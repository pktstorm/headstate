import { fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { Lock, Worktree, WorktreeRepo } from "@/types/pr";
import { useFilters } from "@/store/filters";
import { stubViewport } from "@/test-utils";

const state = vi.hoisted(() => ({
  repos: undefined as WorktreeRepo[] | undefined,
  isLoading: false,
  isError: false,
  classified: undefined as Worktree[] | undefined,
  classifying: false,
  assessed: [] as string[],
  prs: [] as import("@/types/pr").PullRequest[],
  // `number | null` values, not `number`: a null VALUE is a worktree
  // whose walk was abandoned (#769), which is a different fact from an
  // absent KEY meaning "not measured yet".
  sizes: undefined as Map<string, number | null> | undefined,
  // Sizes streamed in while the query is still in flight (#754).
  partialSizes: undefined as Map<string, number | null> | undefined,
  allSizes: undefined as Map<string, number | null> | undefined,
  sizesPending: 0,
  sizesTotal: 0,
  sizesFailed: 0,
  sizing: false,
  // The whole repository's sizing pass rejected (#769). Until then the
  // page never read `isError`, so a rejection showed skeletons and then
  // silently became an em dash.
  sizingFailed: false,
}));

const toastSuccess = vi.hoisted(() => vi.fn());
const toastError = vi.hoisted(() => vi.fn());
// `info` is its own channel, not a success with different words: a prune
// that cleared nothing is neither a failure nor an accomplishment, and
// the page must be able to say so (#793).
const toastInfo = vi.hoisted(() => vi.fn());
vi.mock("sonner", () => ({
  toast: { success: toastSuccess, error: toastError, info: toastInfo },
}));

const dockerImages = vi.hoisted(() => vi.fn(() => [] as unknown[]));
const removeOrphanFn = vi.hoisted(() =>
  vi.fn<(path: string) => Promise<void>>(() => Promise.resolve()),
);
const pullFn = vi.hoisted(() =>
  vi.fn<(path: string) => Promise<string>>(() => Promise.resolve("Already up to date.")),
);
// Typed so the call arguments can be asserted on: the untyped form
// infers an empty tuple, and indexing it is a compile error.
const removeImagesFn = vi.hoisted(() =>
  vi.fn<(ids: string[]) => Promise<{ id: string; error: string | null }[]>>(() =>
    Promise.resolve([]),
  ),
);

vi.mock("../api/hooks", () => ({
  // Idle: the progress line only appears mid-removal.
  useRemovalProgress: () => null,
  useUpdateProgress: () => null,
  useCancelUpdateRun: () => () => Promise.resolve(),
  // Not opened in these tests: the disclosure is closed by default.
  useAssessment: () => ({ data: undefined, isLoading: false }),
  // The cleanup manifest joins worktrees to the images they own, so the
  // page now reads Docker state -- but only while the confirmation is
  // open, which is why the default here is an empty list.
  usePullCheckout: () => pullFn,
  useRemoveOrphan: () => removeOrphanFn,
  // Sizes land one repository at a time on the all-repos view, so the
  // mock carries the progress fields the page renders.
  useAllWorktreeSizes: () => ({
    sizes: state.allSizes ?? new Map<string, number | null>(),
    pending: state.sizesPending ?? 0,
    total: state.sizesTotal ?? 0,
    failed: state.sizesFailed ?? 0,
  }),
  useDockerImages: () => ({ data: dockerImages() }),
  useRemoveImages: () => removeImagesFn,
  useWorktrees: () => ({
    data: state.repos,
    isLoading: state.isLoading,
    isError: state.isError,
    error: "boom",
    refetch: vi.fn(),
  }),
  useWorktreeSafety: () => ({ data: state.classified, isLoading: state.classifying }),
  useRemoveWorktree: () => removeFn,
  useRemoveWorktrees: () => removeManyFn,
  useRemoveWorktreeForced: () => forceFn,
  useUnlockWorktree: () => unlockFn,
  useUnlockWorktrees: () => unlockManyFn,
  usePruneWorktrees: () => pruneFn,
  useAssessed: () => ({ data: state.assessed }),
  useMarkAssessed: () => markAssessedFn,
  useClearAssessed: () => clearAssessedFn,
  usePullRequests: () => ({ data: state.prs }),
  // Mirrors the real hook: a DISABLED query reports `isLoading: true`
  // forever, so the page reads `isFetching` instead -- and a mock that
  // only carried `isLoading` would hide exactly the bug that caused.
  // `partial` mirrors the real hook's stream of per-worktree sizes: the
  // settled `data` is undefined until every tree in the repository has
  // been walked, and rows must fill from `partial` before then (#754).
  useWorktreeSizes: () => ({
    data: state.sizes,
    partial: state.partialSizes ?? new Map<string, number | null>(),
    isLoading: state.sizing,
    isFetching: state.sizing,
    // #769: the page must read this. A mock that omitted it would let
    // the "rejection shows skeletons forever" bug pass unnoticed.
    isError: state.sizingFailed,
  }),
}));

const removeFn = vi.hoisted(() => vi.fn(() => Promise.resolve()));
const forceFn = vi.hoisted(() => vi.fn(() => Promise.resolve()));
const unlockFn = vi.hoisted(() => vi.fn(() => Promise.resolve()));
type Outcome = { path: string; error: string | null };
const removeManyFn = vi.hoisted(() =>
  vi.fn<(repo: string, paths: string[]) => Promise<Outcome[]>>((_r, paths) =>
    Promise.resolve(paths.map((p) => ({ path: p, error: null }))),
  ),
);
// Shaped exactly like `removeManyFn`, because the real bulk unlock is
// shaped like the real bulk removal: an outcome per target, since a row
// somebody else unlocked between the scan and the click is an ordinary
// race and not a reason to abandon the batch (#792).
const unlockManyFn = vi.hoisted(() =>
  vi.fn<(repo: string, paths: string[]) => Promise<Outcome[]>>((_r, paths) =>
    Promise.resolve(paths.map((p) => ({ path: p, error: null }))),
  ),
);
// Resolves with a COUNT, like the real command: `git worktree prune` is
// silent on success, so the number is the only thing that separates
// "cleared 12" from "there was nothing to do" (#793).
const pruneFn = vi.hoisted(() => vi.fn<(repo: string) => Promise<number>>(() => Promise.resolve(0)));

const markAssessedFn = vi.hoisted(() => vi.fn(() => Promise.resolve()));
const clearAssessedFn = vi.hoisted(() => vi.fn(() => Promise.resolve()));
const claudify = vi.hoisted(() =>
  vi.fn(() =>
    Promise.resolve({ command: "cd '/code/proj-a' && claude 'assess'", claude_installed: true }),
  ),
);
vi.mock("../api/tauri", () => ({ claudifyCommand: claudify }));

// The build target, as a mock: `IS_MOBILE_BUILD` is read at module
// scope, and re-importing the component to change it would lose
// every other mock in this file.
const mobileBuild = vi.hoisted(() => ({ current: false }));
vi.mock("@/lib/target", () => ({
  get IS_MOBILE_BUILD() {
    return mobileBuild.current;
  },
  get IS_DESKTOP_BUILD() {
    return !mobileBuild.current;
  },
}));

import { WorktreesPage } from "./WorktreesPage";

/// A lock for the locked-row fixtures (#775).
///
/// Synthetic per CONTRIBUTING.md. `underlying` defaults to `unmerged`
/// so a test that does not mention it cannot accidentally rely on the
/// "would be safe once unlocked" wording.
const lockOf = (over: Partial<Lock> = {}): Lock => ({
  reason: "some tool (pid 123)",
  age_days: 5,
  holder_running: true,
  underlying: { kind: "unmerged" },
  ...over,
});

const wt = (over: Partial<Worktree>): Worktree => ({
  path: "/code/proj-a",
  branch: "feature",
  head: "abc",
  size_bytes: 1024,
  safety: { kind: "unmerged" },
  is_main: false,
  merged_at: null,
  upstream: null,
  last_commit: null,
  ...over,
});

const EMPTY = { "my-prs": {}, "to-review": {}, worktrees: {},
  branches: {}, docker: {}, artifacts: {}, packages: {}, "claude-md": {}, "pr-stats": {}, "system-health": {} } as const;

describe("WorktreesPage on a phone", () => {
  beforeEach(() => {
    dockerImages.mockReturnValue([]);
    Object.assign(state, {
      repos: [{ identity: null, name: "proj", path: "/code/proj", worktrees: [wt({})] }],
      isLoading: false,
      isError: false,
      classified: [wt({ safety: { kind: "safe" } })],
      classifying: false,
      sizes: undefined,
      partialSizes: undefined,
      allSizes: undefined,
      sizesPending: 0,
      sizesTotal: 0,
      sizesFailed: 0,
      sizing: false,
      sizingFailed: false,
      assessed: [],
      prs: [],
    });
    useFilters.setState({
      filtersByView: { ...EMPTY, worktrees: { repo: "/code/proj" } },
      view: "worktrees",
    } as never);
  });
  afterEach(() => stubViewport(null));

  it("stacks a worktree row: name and size, then safety, then the action", () => {
    stubViewport(390);
    render(<WorktreesPage />);
    const branch = screen.getByText("feature");
    const nameLine = branch.parentElement?.parentElement as HTMLElement;
    const size = screen.getByText("1.0 KB");
    expect(size.parentElement).toBe(nameLine);
    const remove = screen.getByRole("button", { name: /^remove$/i });
    // The action sits on its own line, not squeezed beside the path.
    expect(remove.closest("div")).not.toBe(nameLine);
    expect(nameLine.parentElement?.className).toContain("flex-col");
    // The safety verdict is still stated, in full rather than truncated.
    expect(screen.getByText(/safe to delete/i)).toBeTruthy();
    fireEvent.click(remove);
    expect(screen.getByRole("dialog")).toBeTruthy();
  });

  it("keeps the desktop worktree row on one line", () => {
    stubViewport(1400);
    render(<WorktreesPage />);
    const branch = screen.getByText("feature");
    const nameLine = branch.parentElement?.parentElement as HTMLElement;
    expect(screen.getByText("1.0 KB").parentElement).toBe(nameLine);
    expect(screen.getByRole("button", { name: /^remove$/i }).closest("div")).toBe(nameLine);
    expect(nameLine.className).not.toContain("flex-col");
  });

  it("wraps the all-repositories rollup rows so the path gets its own line", () => {
    stubViewport(390);
    useFilters.setState({ filtersByView: { ...EMPTY }, view: "worktrees" } as never);
    render(<WorktreesPage />);
    const row = screen.getByTitle(/open this repository/i);
    expect(row.className).toContain("flex-wrap");
    expect(within(row).getByText("proj")).toBeTruthy();
    expect(within(row).getByText("proj-a")).toBeTruthy();
    expect(within(row).getByText("1.0 KB")).toBeTruthy();
  });
});

describe("WorktreesPage", () => {
  beforeEach(() => {
    // Reset between tests: a leaked image list makes a later assertion
    // about paths fail on a Docker line it never set up.
    dockerImages.mockReturnValue([]);
    removeImagesFn.mockClear();
    // `removeManyFn` is asserted as "not called" by a later test, and a
    // confirm click in an earlier one leaks into it. Cleared here rather
    // than in that test, so every test starts from the same state.
    removeManyFn.mockClear();
    pullFn.mockClear();
    removeOrphanFn.mockClear();
    Object.assign(state, {
      repos: [{ identity: null, name: "proj", path: "/code/proj", worktrees: [wt({})] }],
      isLoading: false,
      isError: false,
      classified: undefined,
      classifying: false,
      sizes: undefined,
      partialSizes: undefined,
      sizing: false,
      // #769. A leaked failure flag turns every later size assertion
      // into "not measured", which is a confusing way to fail.
      sizingFailed: false,
      sizesFailed: 0,
      assessed: [],
      prs: [],
    });
    // These exercise the PER-REPO view, so they select a repo. They used
    // to rely on the `repos?.[0]` fallback -- which silently showed the
    // first repo when no repo was chosen, and is exactly the bug the
    // all-repos rollup replaced.
    useFilters.setState({
      filtersByView: { ...EMPTY, worktrees: { repo: "/code/proj" } },
      view: "worktrees",
      panel: "list",
    });
    // Calls leak between tests otherwise, which makes "was not called"
    // assertions pass or fail depending on ordering.
    removeFn.mockClear();
    toastSuccess.mockClear();
    toastError.mockClear();
    toastInfo.mockClear();
  });

  it("says what it is doing while scanning", () => {
    state.isLoading = true;
    render(<WorktreesPage />);
    expect(screen.getByText(/scanning for worktrees/i)).toBeTruthy();
  });

  it("surfaces a scan failure rather than showing an empty list", () => {
    state.isError = true;
    render(<WorktreesPage />);
    expect(screen.getByText(/could not scan/i)).toBeTruthy();
  });

  // An unconfigured base directory must point at the fix, not read as
  // "you have no worktrees".
  it("points at settings when no repos are found", () => {
    state.repos = [];
    render(<WorktreesPage />);
    expect(screen.getByText(/no repositories found/i)).toBeTruthy();
    expect(screen.getByText(/settings/i)).toBeTruthy();
  });

  it("lists worktrees with their branch and size", () => {
    render(<WorktreesPage />);
    expect(screen.getByText("feature")).toBeTruthy();
    expect(screen.getByText("1.0 KB")).toBeTruthy();
  });

  // Safety is the primary axis: every row must say whether it can be
  // removed and, if not, why.
  it("states why a worktree cannot be removed", () => {
    state.classified = [wt({ safety: { kind: "never_pushed" } })];
    render(<WorktreesPage />);
    expect(screen.getByText(/only here/i)).toBeTruthy();
  });

  it("sorts by size, biggest first", () => {
    state.classified = [
      wt({ path: "/code/small", size_bytes: 1024 }),
      wt({ path: "/code/huge", size_bytes: 5 * 1024 ** 3 }),
    ];
    const { container } = render(<WorktreesPage />);
    const rows = Array.from(container.querySelectorAll(".font-mono")).map(
      (e) => e.textContent ?? "",
    );
    expect(rows[0]).toContain("huge");
  });

  it("says it is still checking before classification lands", () => {
    state.classifying = true;
    render(<WorktreesPage />);
    expect(screen.getByText(/checking what is safe/i)).toBeTruthy();
  });

  // Genuinely disabled, not a warning to click past: 52 of 296 worktrees
  // here hold commits that exist nowhere else.
  // The invariant is unchanged -- nothing unsafe may be removed -- but
  // the row now offers Claudify in that slot rather than a dead Remove,
  // so "no removal is offered" is the assertion rather than "Remove is
  // disabled".
  it("offers no removal at all for anything not provably safe", () => {
    state.classified = [wt({ safety: { kind: "never_pushed" } })];
    render(<WorktreesPage />);
    expect(screen.queryByRole("button", { name: /remove/i })).toBeNull();
  });

  it("enables removal only when safe", () => {
    state.classified = [wt({ safety: { kind: "safe" } })];
    render(<WorktreesPage />);
    expect(screen.getByRole("button", { name: /remove/i })).toHaveProperty("disabled", false);
  });

  // A modal, not an inline banner: with 149 worktrees on one repo the
  // clicked row is far down the page, and a prompt at the top is
  // off-screen -- indistinguishable from nothing happening.
  it("confirms in a dialog naming the path", () => {
    state.classified = [wt({ path: "/code/proj-gone", safety: { kind: "safe" } })];
    render(<WorktreesPage />);
    fireEvent.click(screen.getByRole("button", { name: /^remove$/i }));
    expect(screen.getByRole("dialog")).toBeTruthy();
    expect(screen.getByText("/code/proj-gone")).toBeTruthy();
    expect(removeFn).not.toHaveBeenCalled();
  });

  it("removes only after confirmation", () => {
    state.classified = [wt({ path: "/code/proj-gone", safety: { kind: "safe" } })];
    render(<WorktreesPage />);
    fireEvent.click(screen.getByRole("button", { name: /^remove$/i }));
    const dialog = screen.getByRole("dialog");
    fireEvent.click(within(dialog).getByRole("button", { name: /^remove$/i }));
    expect(removeFn).toHaveBeenCalledWith("/code/proj", "/code/proj-gone");
  });

  it("cancelling removes nothing", () => {
    state.classified = [wt({ safety: { kind: "safe" } })];
    render(<WorktreesPage />);
    fireEvent.click(screen.getByRole("button", { name: /^remove$/i }));
    fireEvent.click(screen.getByRole("button", { name: /cancel/i }));
    expect(removeFn).not.toHaveBeenCalled();
    expect(screen.queryByRole("dialog")).toBeNull();
  });

  // A refusal means the work is still there, which the user must see
  // rather than have hidden behind an optimistic update.
  it("toasts the backend's refusal, message and all", async () => {
    removeFn.mockImplementationOnce(() =>
      Promise.reject("not safe to remove: 3 uncommitted files"),
    );
    state.classified = [wt({ path: "/code/proj-gone", safety: { kind: "safe" } })];
    render(<WorktreesPage />);
    fireEvent.click(screen.getByRole("button", { name: /^remove$/i }));
    fireEvent.click(within(screen.getByRole("dialog")).getByRole("button", { name: /^remove$/i }));
    await waitFor(() =>
      expect(toastError).toHaveBeenCalledWith(
        "Could not remove proj-gone",
        expect.objectContaining({ description: "not safe to remove: 3 uncommitted files" }),
      ),
    );
  });

  // Removal is otherwise silent: the row vanishes on refetch with no
  // confirmation of what happened, which matters when clearing several.
  it("toasts success, naming the worktree", async () => {
    state.classified = [wt({ path: "/code/proj-gone", safety: { kind: "safe" } })];
    render(<WorktreesPage />);
    fireEvent.click(screen.getByRole("button", { name: /^remove$/i }));
    fireEvent.click(within(screen.getByRole("dialog")).getByRole("button", { name: /^remove$/i }));
    await waitFor(() => expect(toastSuccess).toHaveBeenCalledWith("Removed proj-gone"));
  });

  // A merged worktree should say WHEN, so "four months ago" reads
  // differently from "yesterday".
  it("shows the merge date when there is one", () => {
    state.classified = [wt({ safety: { kind: "safe" }, merged_at: "2026-08-18" })];
    render(<WorktreesPage />);
    expect(screen.getByText(/merged 2026-08-18/)).toBeTruthy();
  });

  // The bug: an unclassified row rendered "could not determine: not yet
  // classified" -- a FAILED check, in the same grey as a real failure.
  // Classification takes up to ~57s on a large tree, so that was most of
  // the first minute.
  it("shows a skeleton, not a failure, while a row is still being checked", () => {
    Object.assign(state, {
      repos: [
        { identity: null, name: "proj", path: "/code/proj", worktrees: [wt({ safety: { kind: "pending" } })] },
      ],
      classifying: true,
    });
    render(<WorktreesPage />);
    expect(screen.queryByText(/could not determine/)).toBeNull();
    const row = screen.getByText("proj-a").closest("div") as HTMLElement;
    expect(row.querySelectorAll('[aria-hidden="true"]').length).toBeGreaterThan(0);
  });

  // A row must stop being a skeleton the moment ITS answer lands, rather
  // than waiting for the whole pass to finish.
  it("resolves each row independently as its classification arrives", () => {
    Object.assign(state, {
      repos: [
        {
          identity: null,
          name: "proj",
          path: "/code/proj",
          worktrees: [wt({ path: "/code/proj-a" }), wt({ path: "/code/proj-b" })],
        },
      ],
      classified: [
        wt({ path: "/code/proj-a", safety: { kind: "safe" } }),
        wt({ path: "/code/proj-b", safety: { kind: "pending" } }),
      ],
      classifying: true,
    });
    render(<WorktreesPage />);
    expect(screen.getByText(/safe to delete/)).not.toBeNull();
    const pendingRow = screen.getByText("proj-b").closest("div") as HTMLElement;
    expect(pendingRow.querySelectorAll('[aria-hidden="true"]').length).toBeGreaterThan(0);
  });

  // The em dash read as "measured, and the answer is nothing".
  /// #348/#358: the page read "at least 0 B" with dashes, indefinitely.
  ///
  /// The first fix said "open a repository to measure sizes" -- honest,
  /// but the view exists to answer where the disk went and could not.
  /// It now measures, one repository at a time, and says how many are
  /// outstanding. MEASURED: the full set takes ~2 minutes, so a count
  /// that visibly falls is the difference between "still working" and
  /// "broken".
  it("says how many repositories are still being measured", () => {
    Object.assign(state, {
      repos: [
        { identity: null, name: "a", path: "/code/a", worktrees: [wt({ size_bytes: null })] },
        {
          identity: null,
          name: "b",
          path: "/code/b",
          worktrees: [wt({ path: "/code/b-f", size_bytes: null })],
        },
      ],
      sizesPending: 2,
      sizesTotal: 2,
    });
    useFilters.setState({
      filtersByView: { "my-prs": {}, "to-review": {}, worktrees: {},
  branches: {}, docker: {}, artifacts: {}, packages: {}, "claude-md": {}, "pr-stats": {}, "system-health": {} },
      view: "worktrees",
    } as never);
    render(<WorktreesPage />);
    expect(screen.getByText(/2 of 2 repositories still to go/i)).toBeTruthy();
  });

  /// Silence once everything has answered -- a progress line that never
  /// clears is indistinguishable from one that is stuck.
  it("stops saying it once every repository has answered", () => {
    Object.assign(state, {
      repos: [
        { identity: null, name: "a", path: "/code/a", worktrees: [wt({ size_bytes: 2048 })] },
      ],
      sizesPending: 0,
      sizesTotal: 1,
    });
    useFilters.setState({
      filtersByView: { "my-prs": {}, "to-review": {}, worktrees: {},
  branches: {}, docker: {}, artifacts: {}, packages: {}, "claude-md": {}, "pr-stats": {}, "system-health": {} },
      view: "worktrees",
    } as never);
    render(<WorktreesPage />);
    expect(screen.queryByText(/still to go/i)).toBeNull();
  });

  it("shows a skeleton rather than an em dash while a size is still coming", () => {
    Object.assign(state, {
      repos: [
        {
          identity: null,
          name: "proj",
          path: "/code/proj",
          worktrees: [wt({ size_bytes: null, safety: { kind: "safe" } })],
        },
      ],
      sizing: true,
    });
    render(<WorktreesPage />);
    expect(screen.queryByText("—")).toBeNull();
  });

  /// A worktree the walk gave up on says so, instead of holding a
  /// skeleton for the rest of the pass.
  ///
  /// #769: a repository with 111 worktrees showed every size cell as a
  /// skeleton for 15+ minutes while one with 97 finished in ~10 seconds.
  /// A skeleton is a promise that a number is coming; for a tree the
  /// walk abandoned, no number is coming, and the row has to stop
  /// implying otherwise. `sizing` stays TRUE here on purpose -- the rest
  /// of the repository is still being walked, and a row that has already
  /// given up must not wait for it.
  it("says a worktree could not be measured rather than holding its skeleton", () => {
    Object.assign(state, {
      repos: [
        {
          identity: null,
          name: "proj",
          path: "/code/proj",
          worktrees: [
            wt({ path: "/code/proj/huge", size_bytes: null, safety: { kind: "safe" } }),
          ],
        },
      ],
      // An explicit null VALUE: measured, and the answer is "could not".
      partialSizes: new Map<string, number | null>([["/code/proj/huge", null]]),
      sizing: true,
    });
    render(<WorktreesPage />);
    expect(screen.getByText(/not measured/i)).not.toBeNull();
  });

  /// "Could not measure" must not read as "empty".
  ///
  /// The size column exists to answer "how much do I get back by
  /// deleting this?". Rendering an abandoned walk as 0 B invites
  /// deleting a checkout nobody has measured, which is the worst answer
  /// this column could give.
  it("never renders an unmeasured worktree as zero bytes", () => {
    Object.assign(state, {
      repos: [
        {
          identity: null,
          name: "proj",
          path: "/code/proj",
          worktrees: [
            wt({ path: "/code/proj/huge", size_bytes: null, safety: { kind: "safe" } }),
          ],
        },
      ],
      partialSizes: new Map<string, number | null>([["/code/proj/huge", null]]),
      sizing: true,
    });
    render(<WorktreesPage />);
    expect(screen.queryByText("0 B")).toBeNull();
  });

  /// A sizing pass that REJECTED is surfaced, not swallowed.
  ///
  /// #769: `useWorktreeSizes` exposed `isError` and the page never read
  /// it, so a rejected walk showed skeletons while TanStack retried and
  /// then collapsed to an em dash -- claiming a measurement that never
  /// happened. Here the query has settled (`sizing: false`) and failed,
  /// and every row must say it was not measured.
  it("surfaces a failed sizing pass instead of showing a measured-looking dash", () => {
    Object.assign(state, {
      repos: [
        {
          identity: null,
          name: "proj",
          path: "/code/proj",
          worktrees: [
            wt({ path: "/code/proj/one", size_bytes: null, safety: { kind: "safe" } }),
          ],
        },
      ],
      sizing: false,
      sizingFailed: true,
    });
    render(<WorktreesPage />);
    expect(screen.getByText(/not measured/i)).not.toBeNull();
  });

  /// One abandoned worktree must not take the others' numbers with it.
  ///
  /// The load-bearing guarantee of #769: a parked walk stalled the whole
  /// column at N-1. The rows that DID measure must show their sizes
  /// alongside the one that did not.
  it("keeps showing the other worktrees' sizes when one cannot be measured", () => {
    Object.assign(state, {
      repos: [
        {
          identity: null,
          name: "proj",
          path: "/code/proj",
          worktrees: [
            wt({ path: "/code/proj/huge", size_bytes: null, safety: { kind: "safe" } }),
            wt({ path: "/code/proj/ok", size_bytes: null, safety: { kind: "safe" } }),
          ],
        },
      ],
      partialSizes: new Map<string, number | null>([
        ["/code/proj/huge", null],
        ["/code/proj/ok", 2048],
      ]),
      sizing: true,
    });
    render(<WorktreesPage />);
    expect(screen.getByText(/not measured/i)).not.toBeNull();
    expect(screen.getByText("2.0 KB")).not.toBeNull();
  });

  /// The all-repositories rollup says it too, once the pass is done.
  ///
  /// The same #769 distinction in the other view: while repositories are
  /// still answering, a null is "still coming" and the banner says so.
  /// Once nothing is pending, a null is a walk that was abandoned, and
  /// an em dash there would read as "measured, and the answer is
  /// nothing".
  it("says a worktree was not measured on the all-repositories view", () => {
    Object.assign(state, {
      repos: [
        {
          identity: null,
          name: "proj",
          path: "/code/proj",
          worktrees: [wt({ path: "/code/proj/huge", size_bytes: null })],
        },
      ],
      allSizes: new Map<string, number | null>([["/code/proj/huge", null]]),
      // Nothing left to wait for, so the null is final.
      sizesPending: 0,
      sizesTotal: 1,
    });
    useFilters.setState({
      filtersByView: { ...EMPTY, worktrees: {} },
      view: "worktrees",
    } as never);
    render(<WorktreesPage />);
    expect(screen.getByText(/not measured/i)).not.toBeNull();
  });

  // Safety and size are separate passes; a row whose safety resolved must
  // not be held hostage by a size that has not.
  it("shows a resolved safety even while that row's size is still pending", () => {
    Object.assign(state, {
      repos: [
        {
          identity: null,
          name: "proj",
          path: "/code/proj",
          worktrees: [wt({ size_bytes: null, safety: { kind: "safe" } })],
        },
      ],
      sizing: true,
    });
    render(<WorktreesPage />);
    expect(screen.getByText(/safe to delete/)).not.toBeNull();
  });

  /// A row you are about to click must not move as its number lands.
  ///
  /// #771 replaced the old answer -- refuse to sort at all until every
  /// size was in -- with a sort over what IS known plus a frozen order.
  /// The unmeasured row therefore sorts LAST rather than pinning the
  /// whole list to path order: an unknown size is not a claim of 0 B,
  /// and ranking it as the smallest is what would hide the directory
  /// that might be huge.
  it("sorts on what is known while sizes are still arriving, unknowns last", () => {
    Object.assign(state, {
      repos: [
        {
          identity: null,
          name: "proj",
          path: "/code/proj",
          worktrees: [
            wt({ path: "/code/aaa", size_bytes: null }),
            wt({ path: "/code/zzz", size_bytes: 9_999_999 }),
          ],
        },
      ],
      sizing: true,
    });
    const { container } = render(<WorktreesPage />);
    const names = [...container.querySelectorAll(".font-mono")].map((n) => n.textContent);
    expect(names[0]).toMatch(/^zzz/);
    expect(names[1]).toMatch(/^aaa/);
  });

  it("sorts by size once every size has arrived", () => {
    Object.assign(state, {
      repos: [
        {
          identity: null,
          name: "proj",
          path: "/code/proj",
          worktrees: [
            wt({ path: "/code/aaa", size_bytes: 10 }),
            wt({ path: "/code/zzz", size_bytes: 9_999_999 }),
          ],
        },
      ],
      sizing: false,
    });
    const { container } = render(<WorktreesPage />);
    const names = [...container.querySelectorAll(".font-mono")].map((n) => n.textContent);
    expect(names[0]).toMatch(/^zzz/);
  });

  // #754: a row's own size is known long before the repository's
  // slowest tree has been walked, and there is no reason to withhold it.
  // Before the fix this view read only the SETTLED query, so every row
  // held a skeleton for the whole walk -- minutes on a repository with a
  // 200 GB checkout, which is what "flashing skeletons indefinitely"
  // was.
  it("shows a size that has streamed in before the whole repository is measured", () => {
    Object.assign(state, {
      repos: [
        {
          identity: null,
          name: "proj",
          path: "/code/proj",
          worktrees: [
            wt({ path: "/code/done", size_bytes: null }),
            wt({ path: "/code/slow", size_bytes: null }),
          ],
        },
      ],
      // The query has NOT settled -- this is the state that used to show
      // two skeletons and nothing else.
      sizes: undefined,
      // Exactly 2.5 GiB, so the rendered string is unambiguous.
      partialSizes: new Map([["/code/done", 2.5 * 1024 ** 3]]),
      sizing: true,
    });
    render(<WorktreesPage />);
    expect(screen.getByText("2.5 GB")).not.toBeNull();
  });

  // The other half of #754: whatever the measurement costs, the view has
  // to say how far along it is. "measuring sizes…" on its own is
  // indistinguishable from a hang once it has said it for ten minutes.
  it("counts down the worktrees still to be measured", () => {
    Object.assign(state, {
      repos: [
        {
          identity: null,
          name: "proj",
          path: "/code/proj",
          worktrees: [
            wt({ path: "/code/a", size_bytes: null }),
            wt({ path: "/code/b", size_bytes: null }),
            wt({ path: "/code/c", size_bytes: null }),
          ],
        },
      ],
      sizes: undefined,
      partialSizes: new Map([["/code/a", 1_000]]),
      sizing: true,
    });
    render(<WorktreesPage />);
    // One of three has landed, so two remain -- a number that falls,
    // rather than a spinner that says only "something is happening".
    expect(screen.getByText(/2 of 3 to go/)).not.toBeNull();
  });

  // Deleting on an unresolved verdict is the one unrecoverable mistake
  // this page can make.
  it("refuses to offer removal while a row is still being checked", () => {
    Object.assign(state, {
      repos: [
        { identity: null, name: "proj", path: "/code/proj", worktrees: [wt({ safety: { kind: "pending" } })] },
      ],
      classifying: true,
    });
    render(<WorktreesPage />);
    const btn = screen.getByRole("button", { name: "Remove" }) as HTMLButtonElement;
    expect(btn.disabled).toBe(true);
    fireEvent.click(btn);
    expect(screen.queryByRole("dialog")).toBeNull();
  });

  // 124 of 268 worktrees on a real machine cannot be removed. The row
  // used to show a dead Remove there; it now answers the question that
  // actually applies -- is there anything in here worth keeping?
  describe("Claudify", () => {
    it.each([["never_pushed"], ["unmerged"], ["dirty"], ["unpushed"], ["empty"]])(
      "offers it for %s",
      (kind) => {
        state.classified = [wt({ safety: { kind } as Worktree["safety"] })];
        const r = render(<WorktreesPage />);
        expect(screen.getByRole("button", { name: /claudify/i })).toBeTruthy();
        r.unmount();
      },
    );

    it("does not offer it where Remove already applies", () => {
      state.classified = [wt({ safety: { kind: "safe" } })];
      render(<WorktreesPage />);
      expect(screen.queryByRole("button", { name: /claudify/i })).toBeNull();
      expect(screen.getByRole("button", { name: /remove/i })).toBeTruthy();
    });

    /// The phone has no terminal to paste into, and `copyText` reports
    /// "no clipboard access" in a non-secure webview context anyway --
    /// so on the mobile build the command is SHOWN instead of copied.
    it("shows the command on the mobile build rather than copying it", async () => {
      mobileBuild.current = true;
      state.classified = [wt({ safety: { kind: "dirty", detail: 2 } })];
      const writeText = vi.fn(() => Promise.resolve());
      Object.assign(navigator, { clipboard: { writeText } });

      render(<WorktreesPage />);
      fireEvent.click(screen.getByRole("button", { name: /claudify/i }));

      // The command itself, readable, and never sent to a clipboard
      // that would have refused it.
      expect(await screen.findByText(/claude 'assess'/)).toBeTruthy();
      expect(writeText).not.toHaveBeenCalled();
      mobileBuild.current = false;
    });

    /// The assessment action used to live inside the clipboard's
    /// success branch, and a failed copy took an early return -- so the
    /// ONLY route to "Remove anyway…" disappeared whenever the
    /// clipboard was unavailable. That is every phone, and any desktop
    /// whose window is not focused.
    it("still offers the assessment action when the copy fails", async () => {
      state.classified = [wt({ safety: { kind: "dirty", detail: 2 } })];
      Object.assign(navigator, {
        clipboard: { writeText: () => Promise.reject(new Error("denied")) },
      });

      render(<WorktreesPage />);
      fireEvent.click(screen.getByRole("button", { name: /claudify/i }));
      await waitFor(() => expect(toastError).toHaveBeenCalled());

      const opts = toastError.mock.calls.at(-1)?.[1] as { action?: { label: string } } | undefined;
      expect(opts?.action?.label).toMatch(/read the assessment/i);
    });

    // Offering an action based on a verdict that has not arrived is the
    // bug #190 was.
    it("does not offer it while the row is still being classified", () => {
      state.classified = [wt({ safety: { kind: "pending" } })];
      state.classifying = true;
      const r = render(<WorktreesPage />);
      expect(screen.queryByRole("button", { name: /claudify/i })).toBeNull();
      r.unmount();
      state.classifying = false;
    });

    it("copies the command and says where to paste it", async () => {
      const writeText = vi.fn<(text: string) => Promise<void>>(() => Promise.resolve());
      Object.assign(navigator, { clipboard: { writeText } });
      state.classified = [wt({ safety: { kind: "never_pushed" } })];
      render(<WorktreesPage />);

      fireEvent.click(screen.getByRole("button", { name: /claudify/i }));
      await waitFor(() => expect(writeText).toHaveBeenCalled());
      expect(writeText.mock.calls[0][0]).toContain("claude");
      expect(toastSuccess).toHaveBeenCalled();
      const [, opts] = toastSuccess.mock.calls[0] as [string, { description: string }];
      expect(opts.description).toMatch(/paste it in your terminal/i);
    });

    /// #393: the toast must SAY the clipboard, not just "copied".
    it("names the clipboard in the confirmation", async () => {
      const writeText = vi.fn<(text: string) => Promise<void>>(() => Promise.resolve());
      Object.assign(navigator, { clipboard: { writeText } });
      state.classified = [wt({ safety: { kind: "never_pushed" } })];
      render(<WorktreesPage />);

      fireEvent.click(screen.getByRole("button", { name: /claudify/i }));
      await waitFor(() => expect(toastSuccess).toHaveBeenCalled());
      expect(toastSuccess.mock.calls[0][0]).toMatch(/clipboard/i);
    });

    /// #393: copying a prompt must not unlock the force-remove path.
    ///
    /// It used to mark the worktree assessed as a side effect, which
    /// armed "Remove anyway…" on a worktree nobody had read a verdict
    /// for -- and swapped a narrow button for a wide one seconds later,
    /// re-flowing every column in the table.
    it("does not mark the worktree assessed just for copying", async () => {
      const writeText = vi.fn<(text: string) => Promise<void>>(() => Promise.resolve());
      Object.assign(navigator, { clipboard: { writeText } });
      state.classified = [wt({ safety: { kind: "never_pushed" } })];
      render(<WorktreesPage />);

      fireEvent.click(screen.getByRole("button", { name: /claudify/i }));
      await waitFor(() => expect(toastSuccess).toHaveBeenCalled());
      expect(markAssessedFn).not.toHaveBeenCalled();
    });

    /// ...and the toast offers the deliberate way to record it.
    it("offers an explicit way to say the assessment was read", async () => {
      const writeText = vi.fn<(text: string) => Promise<void>>(() => Promise.resolve());
      Object.assign(navigator, { clipboard: { writeText } });
      state.classified = [wt({ safety: { kind: "never_pushed" } })];
      render(<WorktreesPage />);

      fireEvent.click(screen.getByRole("button", { name: /claudify/i }));
      await waitFor(() => expect(toastSuccess).toHaveBeenCalled());
      const [, opts] = toastSuccess.mock.calls[0] as [
        string,
        { action?: { label: string; onClick: () => void } },
      ];
      expect(opts.action?.label).toMatch(/read the assessment/i);

      opts.action?.onClick();
      await waitFor(() => expect(markAssessedFn).toHaveBeenCalled());
    });

    /// Marking an assessment used to be a ONE-WAY DOOR: Claudify was
    /// replaced by "Remove anyway…", the mark persisted across restarts,
    /// and only a moved branch cleared it. One exploratory click removed
    /// the only route to that worktree's prompt.
    it("still offers Claudify after the assessment is marked", async () => {
      const writeText = vi.fn<(text: string) => Promise<void>>(() => Promise.resolve());
      Object.assign(navigator, { clipboard: { writeText } });
      const w = wt({ safety: { kind: "never_pushed" } });
      state.classified = [w];
      state.assessed = [w.path];
      render(<WorktreesPage />);

      // The row now offers the forced removal...
      expect(screen.getByRole("button", { name: /remove anyway/i })).toBeTruthy();
      // ...and Claudify is still reachable, behind the kebab.
      fireEvent.click(screen.getByRole("button", { name: /more actions/i }));
      fireEvent.click(screen.getByRole("menuitem", { name: /copy the claudify command/i }));
      await waitFor(() => expect(writeText).toHaveBeenCalled());
    });

    it("can forget an assessment, restoring the plain Claudify button", async () => {
      const w = wt({ safety: { kind: "never_pushed" } });
      state.classified = [w];
      state.assessed = [w.path];
      render(<WorktreesPage />);

      fireEvent.click(screen.getByRole("button", { name: /more actions/i }));
      fireEvent.click(screen.getByRole("menuitem", { name: /forget the assessment/i }));
      await waitFor(() => expect(clearAssessedFn).toHaveBeenCalledWith(w.path));
    });

    /// The kebab is on every row from #770 onwards, but the
    /// Claudify/Forget PAIR inside it is still only for the assessed
    /// state: an unassessed row already has Claudify as its one action,
    /// and a menu holding a duplicate of the button beside it would be
    /// noise. What the menu does carry unconditionally is removal --
    /// that is the affordance the toast used to own.
    it("omits the assessment items from the kebab until one is marked", () => {
      state.classified = [wt({ safety: { kind: "never_pushed" } })];
      state.assessed = [];
      render(<WorktreesPage />);
      expect(screen.getByRole("button", { name: /claudify/i })).toBeTruthy();
      fireEvent.click(screen.getByRole("button", { name: /more actions/i }));
      expect(screen.queryByRole("menuitem", { name: /copy the claudify command/i })).toBeNull();
      expect(screen.queryByRole("menuitem", { name: /forget the assessment/i })).toBeNull();
      expect(screen.getByRole("menuitem", { name: /remove worktree/i })).toBeTruthy();
    });

    /// #396: an ABSENT clipboard produced NO toast at all.
    ///
    /// `navigator.clipboard.writeText(...)` throws synchronously on
    /// property access when the object is missing, so `.then(ok, err)`
    /// attached neither handler and the click looked inert -- which is
    /// exactly what was reported against v4.0.0, after #393 had already
    /// fixed the assessment mark and the toast wording.
    it("says so when the window has no clipboard at all", async () => {
      Object.assign(navigator, { clipboard: undefined });
      state.classified = [wt({ safety: { kind: "never_pushed" } })];
      render(<WorktreesPage />);

      fireEvent.click(screen.getByRole("button", { name: /claudify/i }));
      await waitFor(() => expect(toastError).toHaveBeenCalled());
      expect(toastError.mock.calls[0][0]).toMatch(/could not copy/i);
      const [, opts] = toastError.mock.calls[0] as [string, { description: string }];
      expect(opts.description).toMatch(/no clipboard access/i);
    });

    /// #347: reported as "no indication it copied anything". The
    /// success toast is asserted above, so the visible gap is the
    /// FAILURE path -- `navigator.clipboard` rejects when the document
    /// is not focused, which is a real case in a desktop webview, and
    /// nothing tested that the user hears about it.
    it("says so when the clipboard refuses", async () => {
      const writeText = vi.fn<(text: string) => Promise<void>>(() =>
        Promise.reject(new Error("Document is not focused")),
      );
      Object.assign(navigator, { clipboard: { writeText } });
      state.classified = [wt({ safety: { kind: "never_pushed" } })];
      render(<WorktreesPage />);

      fireEvent.click(screen.getByRole("button", { name: /claudify/i }));
      await waitFor(() => expect(toastError).toHaveBeenCalled());
      expect(toastError.mock.calls[0][0]).toMatch(/could not copy/i);
      // And the reason, which is the actionable part -- "could not
      // copy" alone leaves the user with nothing to do.
      const [, opts] = toastError.mock.calls[0] as [string, { description?: string }];
      expect(opts?.description).toMatch(/not focused/i);
    });

    // Better to learn it here than as `command not found` after pasting.
    it("says so when Claude Code was not found, but still copies", async () => {
      const writeText = vi.fn<(text: string) => Promise<void>>(() => Promise.resolve());
      Object.assign(navigator, { clipboard: { writeText } });
      claudify.mockResolvedValueOnce({ command: "cd x && claude y", claude_installed: false });
      state.classified = [wt({ safety: { kind: "unmerged" } })];
      render(<WorktreesPage />);

      fireEvent.click(screen.getByRole("button", { name: /claudify/i }));
      await waitFor(() => expect(writeText).toHaveBeenCalled());
      const [, opts] = toastSuccess.mock.calls[0] as [string, { description: string }];
      expect(opts.description).toMatch(/not found/i);
    });
  });

  // Removal takes a moment, and a button that still looks live invites a
  // second click on a directory that is already being deleted.
  describe("removal feedback", () => {
    it("shows the button as busy and stops accepting clicks", async () => {
      let release: () => void = () => {};
      removeFn.mockImplementationOnce(
        () => new Promise<void>((r) => { release = r; }),
      );
      state.classified = [wt({ safety: { kind: "safe" } })];
      render(<WorktreesPage />);

      fireEvent.click(screen.getByRole("button", { name: /^remove$/i }));
      fireEvent.click(
        within(screen.getByRole("dialog")).getByRole("button", { name: /^remove$/i }),
      );

      const busy = await screen.findByRole("button", { name: /removing/i });
      expect((busy as HTMLButtonElement).disabled).toBe(true);

      // A second click while in flight must not submit again.
      fireEvent.click(busy);
      expect(removeFn).toHaveBeenCalledTimes(1);

      release();
      await waitFor(() => expect(toastSuccess).toHaveBeenCalled());
    });

    // The backend re-checks safety at delete time and can refuse. The
    // button must come back, not stay stuck on "Removing...".
    it("returns the button to normal when the removal is refused", async () => {
      removeFn.mockRejectedValueOnce("not safe to remove: 2 uncommitted files");
      state.classified = [wt({ safety: { kind: "safe" } })];
      render(<WorktreesPage />);

      fireEvent.click(screen.getByRole("button", { name: /^remove$/i }));
      fireEvent.click(
        within(screen.getByRole("dialog")).getByRole("button", { name: /^remove$/i }),
      );

      await waitFor(() => expect(toastError).toHaveBeenCalled());
      const back = await screen.findByRole("button", { name: /^remove$/i });
      expect((back as HTMLButtonElement).disabled).toBe(false);
    });

    // With 100+ rows, freezing all of them because one is in flight
    // would be worse than the current behaviour.
    it("leaves other rows clickable while one is being removed", async () => {
      let release: () => void = () => {};
      removeFn.mockImplementationOnce(
        () => new Promise<void>((r) => { release = r; }),
      );
      state.classified = [
        wt({ path: "/code/proj-a", safety: { kind: "safe" } }),
        wt({ path: "/code/proj-b", safety: { kind: "safe" } }),
      ];
      render(<WorktreesPage />);

      fireEvent.click(screen.getAllByRole("button", { name: /^remove$/i })[0]);
      fireEvent.click(
        within(screen.getByRole("dialog")).getByRole("button", { name: /^remove$/i }),
      );
      await screen.findByRole("button", { name: /removing/i });

      const others = screen.getAllByRole("button", { name: /^remove$/i });
      expect(others).toHaveLength(1);
      expect((others[0] as HTMLButtonElement).disabled).toBe(false);

      release();
    });
  });

  // How much work is in a branch and how stale it is are the two facts
  // that decide what to do with a worktree you do not recognise.
  describe("row facts", () => {
    it("shows ahead and behind compactly", () => {
      state.classified = [
        wt({ path: "/code/a", upstream: { kind: "ahead", n: 3 } }),
        wt({ path: "/code/b", upstream: { kind: "behind", n: 7 } }),
        wt({ path: "/code/c", upstream: { kind: "diverged", n: [2, 5] } }),
      ];
      render(<WorktreesPage />);
      const text = document.body.textContent ?? "";
      expect(text).toContain("↑3");
      expect(text).toContain("↓7");
      expect(text).toContain("↑2 ↓5");
    });

    // The difference between "this is redundant" and "this is the only
    // copy" is worth a word, where "up to date" is just noise.
    it("names a local-only branch but stays quiet when up to date", () => {
      state.classified = [wt({ upstream: { kind: "untracked" } })];
      const r = render(<WorktreesPage />);
      expect(document.body.textContent).toContain("local only");
      r.unmount();

      state.classified = [wt({ upstream: { kind: "current" } })];
      render(<WorktreesPage />);
      expect(document.body.textContent).not.toContain("up to date");
    });

    it("shows how stale the work is, relatively", () => {
      // relativeTime has no weeks tier -- days up to 30, then months.
      const twoWeeks = new Date(Date.now() - 14 * 864e5).toISOString();
      state.classified = [wt({ last_commit: twoWeeks })];
      render(<WorktreesPage />);
      expect(document.body.textContent).toContain("14 days ago");
    });

    // merged_at says whether the work is accounted for; last_commit says
    // how old it is. A branch written in March and merged in August has
    // both, and showing one for the other misleads.
    it("does not confuse the last commit date with the merge date", () => {
      state.classified = [
        wt({
          safety: { kind: "safe" },
          merged_at: "2026-08-01",
          last_commit: new Date(Date.now() - 90 * 864e5).toISOString(),
        }),
      ];
      render(<WorktreesPage />);
      const body = document.body.textContent ?? "";
      expect(body).toContain("merged 2026-08-01");
      expect(body).toContain("3 months ago");
    });

    /// The longest verdict this page can actually produce must not cost
    /// the row its name or its actions (#818).
    ///
    /// The reported row was a lock: `lockReason` wraps git's own free
    /// text -- here the real string from the issue, pid and start time
    /// and all -- and then appends both of its own clauses, so this
    /// fixture is the worst case rather than a long string invented to
    /// make a point. It used to push the name out of view to the left
    /// and the Remove button and kebab out of the bordered box to the
    /// right, because the verdict cell was `shrink-0` with no width
    /// bound and the name was the only cell that could give.
    ///
    /// Asserted on PRESENCE, not on pixels: jsdom does no layout, so
    /// there is no width here to measure. What a unit test can pin is
    /// that the fix is structural -- the name and the actions are still
    /// rendered, the verdict carries `truncate` plus a `title` so the
    /// clipped tail is still reachable, and the list clips rather than
    /// letting content escape. Those are the four things that regressed
    /// in the bug and the four a careless class change would undo.
    it("keeps the name and the actions when the verdict is as long as it gets", () => {
      const longest = lockOf({
        age_days: 0,
        // Verbatim from #818, and the shape our own agents write.
        reason:
          "claude agent agent-a0cc35ddcbc894eda (pid 14779 start Fri Sep 11 09:43:48 2026)",
        holder_running: false,
        // Earns the second appended clause -- "merged, would be safe
        // once unlocked" -- so both of `lockReason`'s additions are in
        // play, not just one.
        underlying: { kind: "safe" },
      });
      state.classified = [
        wt({ path: "/code/agent-a0cc35ddcbc894eda", safety: { kind: "locked", detail: longest } }),
      ];
      const { container } = render(<WorktreesPage />);

      // The row still says WHICH worktree it is.
      expect(screen.getByText(/^agent-a0cc35ddcbc894eda/)).toBeTruthy();
      // ...and still offers the way to act on it. A locked row's action
      // is the kebab, which is the control the issue reported escaping
      // the table alongside Remove.
      expect(screen.getByRole("button", { name: /more actions/i })).toBeTruthy();

      // The verdict yields instead of the name: it truncates, and the
      // whole sentence survives in the tooltip. Without the `title`
      // this assertion would pass while the tail was simply lost, which
      // is the failure mode the issue called out by name.
      const verdict = container.querySelector(".truncate[title]") as HTMLElement;
      expect(verdict).toBeTruthy();
      expect(verdict.title).toContain("pid 14779");
      expect(verdict.title).toContain("holder process is gone");
      expect(verdict.title).toContain("merged, would be safe once unlocked");

      // The backstop: a cell that somehow still overflows is clipped at
      // the border rather than rendered outside it.
      expect(container.querySelector(".overflow-hidden.rounded-md")).toBeTruthy();
    });
  });

  // 106 of 268 worktrees are safe on a real machine, mostly concentrated
  // in a few repos. Clicking each adds no safety, only clicks.
  /// The main checkout is not a peer of the rows below it: every one of
  /// those is a removal candidate and it never is. Its row also carries
  /// the upstream prose that explains why the others are stale.
  it("pins the main checkout above every other row", () => {
    state.classified = [
      // Ordered so main loses on every OTHER key: it is smallest, and
      // sorts last by path.
      wt({ path: "/code/zzz-b", size_bytes: 9_000_000, safety: { kind: "safe" } }),
      wt({ path: "/code/aaa-a", size_bytes: 5_000_000, safety: { kind: "safe" } }),
      wt({
        path: "/code/zzz-main",
        size_bytes: 1,
        is_main: true,
        safety: { kind: "main_checkout" },
      }),
    ];
    render(<WorktreesPage />);
    // Rows show the basename, not the full path, and the element also
    // carries the branch -- so match the leading name rather than the
    // whole string.
    const rendered = screen.getAllByText(/^(zzz-b|aaa-a|zzz-main)/);
    expect(rendered[0].textContent).toMatch(/^zzz-main/);
  });

  /// #340: the main checkout reported how far behind it was and offered
  /// no way to act on it, so fixing it meant leaving the app.
  describe("updating the main checkout", () => {
    const withMain = (safety: unknown = { kind: "main_checkout" }) => [
      wt({ path: "/code/proj", is_main: true, safety: safety as never }),
      wt({ path: "/code/proj-feature", safety: { kind: "safe" } }),
    ];

    it("offers the update only on the main checkout", () => {
      state.classified = withMain();
      render(<WorktreesPage />);
      expect(screen.getAllByRole("button", { name: /update to latest/i })).toHaveLength(1);
    });

    it("pulls that checkout", async () => {
      state.classified = withMain();
      render(<WorktreesPage />);
      fireEvent.click(screen.getByRole("button", { name: /update to latest/i }));
      await waitFor(() => expect(pullFn).toHaveBeenCalledWith("/code/proj"));
    });

    /// Disabled rather than hidden, with the reason in the title: an
    /// absent button just looks broken, while a greyed one that says
    /// "3 uncommitted files" teaches. The Rust side refuses too -- this
    /// is the explanation, not the gate.
    ///
    /// The state below is SYNTHETIC: `classify` returns `MainCheckout`
    /// for the main checkout before it reads `git status`, so a real
    /// scan never pairs `is_main` with a dirty verdict. The gate a user
    /// meets is `pull_checkout`'s own check, which ignores untracked
    /// files (#653).
    it("refuses a dirty checkout and says how dirty", () => {
      state.classified = withMain({ kind: "dirty", detail: 3 });
      render(<WorktreesPage />);
      const btn = screen.getByRole("button", { name: /update to latest/i });
      expect(btn).toHaveProperty("disabled", true);
      expect(btn.getAttribute("title")).toMatch(/3 uncommitted files/);
    });

    /// A real fast-forward is summarised, not shown whole.
    ///
    /// The reported bug (#652): a busy repository's diffstat is hundreds
    /// of lines, and all of them went into the toast.
    it("summarises a fast-forward instead of dumping the diffstat", async () => {
      state.classified = withMain();
      pullFn.mockResolvedValueOnce(
        "Updating a1b2c3d..e4f5a6b\nFast-forward\n" +
          Array.from({ length: 300 }, (_, i) => ` src/f${i}.ts | 3 ++-`).join("\n") +
          "\n 300 files changed, 900 insertions(+), 300 deletions(-)\n",
      );
      render(<WorktreesPage />);
      fireEvent.click(screen.getByRole("button", { name: /update to latest/i }));
      await waitFor(() => expect(toastSuccess).toHaveBeenCalled());
      const shown = toastSuccess.mock.calls[0][0] as string;
      expect(shown.split("\n")).toHaveLength(1);
      expect(shown).toContain("300 files changed");
      expect(shown).not.toContain("src/f0.ts");
    });

    /// Git says "Already up to date." when there was nothing to fetch,
    /// which is a real answer -- replacing it with a claim that
    /// something changed would be a small lie.
    it("reports git's own words on success", async () => {
      state.classified = withMain();
      pullFn.mockResolvedValueOnce("Fast-forward to 3 commits");
      render(<WorktreesPage />);
      fireEvent.click(screen.getByRole("button", { name: /update to latest/i }));
      await waitFor(() => expect(toastSuccess).toHaveBeenCalled());
      expect(toastSuccess.mock.calls[0][0]).toContain("Fast-forward");
    });

    it("reports git's own refusal on failure", async () => {
      state.classified = withMain();
      pullFn.mockRejectedValueOnce("divergent branches");
      render(<WorktreesPage />);
      fireEvent.click(screen.getByRole("button", { name: /update to latest/i }));
      await waitFor(() => expect(toastError).toHaveBeenCalled());
      expect(toastError.mock.calls[0][1]).toMatchObject({
        description: "divergent branches",
      });
    });
  });

  /// Reported: the orphan row said "its repository is gone" and the
  /// Remove button could not be clicked -- so the user was told about
  /// 2.5 GB they could not act on.
  describe("orphaned worktrees", () => {
    const orphan = () =>
      wt({ path: "/code/veil-coh", safety: { kind: "orphaned" } as never });

    it("offers Delete rather than a disabled Remove", () => {
      state.classified = [orphan()];
      render(<WorktreesPage />);
      const btn = screen.getByRole("button", { name: /delete/i }) as HTMLButtonElement;
      expect(btn.disabled).toBe(false);
    });

    /// A DIFFERENT call from the ordinary removal: git cannot remove a
    /// worktree whose repository is gone, so this deletes the
    /// directory after re-checking on the Rust side.
    it("deletes through the orphan path, not the worktree path", async () => {
      state.classified = [orphan()];
      render(<WorktreesPage />);
      fireEvent.click(screen.getByRole("button", { name: /delete/i }));
      await waitFor(() => expect(removeOrphanFn).toHaveBeenCalledWith("/code/veil-coh"));
      expect(removeFn).not.toHaveBeenCalled();
    });

    it("reports a refusal in the Rust side's own words", async () => {
      state.classified = [orphan()];
      removeOrphanFn.mockRejectedValueOnce("this is no longer an orphaned worktree");
      render(<WorktreesPage />);
      fireEvent.click(screen.getByRole("button", { name: /delete/i }));
      await waitFor(() => expect(toastError).toHaveBeenCalled());
      expect(toastError.mock.calls[0][1]).toMatchObject({
        description: "this is no longer an orphaned worktree",
      });
    });

    /// An orphan must never reach the bulk path: it is precisely the
    /// case where the delete-time safety re-check cannot run.
    it("is not counted among the safe worktrees", () => {
      state.classified = [orphan(), wt({ path: "/code/ok", safety: { kind: "safe" } })];
      render(<WorktreesPage />);
      expect(screen.queryByRole("button", { name: /remove 2 safe/i })).toBeNull();
    });
  });

  describe("bulk removal", () => {
    const threeSafe = () => [
      wt({ path: "/code/a", safety: { kind: "safe" }, size_bytes: 1024 }),
      wt({ path: "/code/b", safety: { kind: "safe" }, size_bytes: 2048 }),
      wt({ path: "/code/c", safety: { kind: "never_pushed" } }),
    ];

    /// #268: the cleanup that spans three systems. A merged branch
    /// leaves a worktree on disk AND a Docker image built from it, and
    /// removing the second used to mean going to another view and
    /// working out by hand which images belonged to what.
    // The Docker-image half of this dialog was REMOVED, not broken.
    //
    // It claimed images built from each worktree, and that join could
    // never fire: every build context on a real machine is the MAIN
    // checkout, which `Safety::MainCheckout` excludes from the manifest
    // by construction. Measured in #336 -- 50 build records, 2 distinct
    // contexts, both the main checkout. The tests that lived here
    // asserted a feature that had matched nothing since it shipped.

    /// Reported: the modal stayed open after a successful bulk removal,
    /// with no sign anything had happened, while the count silently
    /// ticked down behind it.
    it("closes the dialog after a successful removal", async () => {
      state.classified = threeSafe();
      removeManyFn.mockResolvedValueOnce([
        { path: "/code/a", error: null },
        { path: "/code/b", error: null },
      ]);
      render(<WorktreesPage />);
      fireEvent.click(screen.getByRole("button", { name: /remove 2 safe worktrees/i }));
      fireEvent.click(screen.getByRole("button", { name: /^remove 2 worktrees$/i }));
      await waitFor(() => expect(screen.queryByRole("dialog")).toBeNull());
    });

    /// #477: the dialog must close when the removal STARTS, not when it
    /// finishes.
    ///
    /// Removing ~100 worktrees is around 30 seconds, and the modal sat
    /// over the whole app for all of it. The two tests either side of
    /// this one only assert the dialog closes eventually, which the old
    /// code also did -- this one holds the promise open and asserts the
    /// dialog is already gone while the work is still running.
    it("closes the dialog while the removal is still running", async () => {
      state.classified = threeSafe();
      let finish!: (v: { path: string; error: string | null }[]) => void;
      removeManyFn.mockReturnValueOnce(
        new Promise((resolve) => {
          finish = resolve;
        }),
      );
      render(<WorktreesPage />);
      fireEvent.click(screen.getByRole("button", { name: /remove 2 safe worktrees/i }));
      fireEvent.click(screen.getByRole("button", { name: /^remove 2 worktrees$/i }));

      // Still running -- and the dialog is already gone.
      await waitFor(() => expect(screen.queryByRole("dialog")).toBeNull());
      expect(removeManyFn).toHaveBeenCalledTimes(1);

      // And the work still completes and reports, unblocked.
      finish([
        { path: "/code/a", error: null },
        { path: "/code/b", error: null },
      ]);
      await waitFor(() => expect(toastSuccess).toHaveBeenCalled());
    });

    /// The error is in a toast; a modal left open on top of it hides the
    /// message that explains what went wrong.
    it("closes the dialog when the removal fails outright", async () => {
      state.classified = threeSafe();
      removeManyFn.mockRejectedValueOnce("git exploded");
      render(<WorktreesPage />);
      fireEvent.click(screen.getByRole("button", { name: /remove 2 safe worktrees/i }));
      fireEvent.click(screen.getByRole("button", { name: /^remove 2 worktrees$/i }));
      await waitFor(() => expect(screen.queryByRole("dialog")).toBeNull());
    });

    /// `bulkBusy` existed and nothing read it, so a slow removal showed
    /// an unchanging button and looked inert.
    it("says it is working while the removal runs", async () => {
      state.classified = threeSafe();
      let settle: (v: { path: string; error: string | null }[]) => void = () => {};
      removeManyFn.mockImplementationOnce(() => new Promise((res) => { settle = res; }));
      render(<WorktreesPage />);
      fireEvent.click(screen.getByRole("button", { name: /remove 2 safe worktrees/i }));
      fireEvent.click(screen.getByRole("button", { name: /^remove 2 worktrees$/i }));

      await screen.findByRole("button", { name: /removing/i });
      settle([{ path: "/code/a", error: null }, { path: "/code/b", error: null }]);
      await waitFor(() => expect(screen.queryByRole("dialog")).toBeNull());
    });

    it("counts only the safe rows, in the label", () => {
      state.classified = threeSafe();
      render(<WorktreesPage />);
      expect(screen.getByRole("button", { name: /remove 2 safe worktrees/i })).toBeTruthy();
    });

    it("lists every path in the confirmation, not just a count", () => {
      state.classified = threeSafe();
      render(<WorktreesPage />);
      fireEvent.click(screen.getByRole("button", { name: /remove 2 safe worktrees/i }));
      const dialog = screen.getByRole("dialog");
      expect(within(dialog).getByText("/code/a")).toBeTruthy();
      expect(within(dialog).getByText("/code/b")).toBeTruthy();
      // The unsafe one must not be in the list at all.
      expect(within(dialog).queryByText("/code/c")).toBeNull();
      expect(removeManyFn).not.toHaveBeenCalled();
    });

    // Never unmerged, never_pushed, dirty, or unpushed -- regardless of
    // what any assessment said. Bulk is for the provably-safe set only.
    it("never submits a worktree that is not safe", async () => {
      state.classified = threeSafe();
      render(<WorktreesPage />);
      fireEvent.click(screen.getByRole("button", { name: /remove 2 safe worktrees/i }));
      fireEvent.click(
        within(screen.getByRole("dialog")).getByRole("button", { name: /remove 2 worktrees/i }),
      );
      await waitFor(() => expect(removeManyFn).toHaveBeenCalled());
      // Sorted by size for display, so compare the set rather than the
      // order -- what matters is which worktrees were submitted.
      expect([...removeManyFn.mock.calls[0][1]].sort()).toEqual(["/code/a", "/code/b"]);
    });

    // Partial failure is the normal case: safety is re-checked at delete
    // time, so a worktree that went dirty since the scan is refused.
    it("reports partial failure rather than a bare success", async () => {
      removeManyFn.mockResolvedValueOnce([
        { path: "/code/a", error: null },
        { path: "/code/b", error: "not safe to remove: 2 uncommitted files" },
      ]);
      state.classified = threeSafe();
      render(<WorktreesPage />);
      fireEvent.click(screen.getByRole("button", { name: /remove 2 safe worktrees/i }));
      fireEvent.click(
        within(screen.getByRole("dialog")).getByRole("button", { name: /remove 2 worktrees/i }),
      );

      await waitFor(() => expect(toastError).toHaveBeenCalled());
      expect(toastSuccess).not.toHaveBeenCalled();
      const [title, opts] = toastError.mock.calls[0] as [string, { description: string }];
      expect(title).toMatch(/1 of 2/);
      expect(opts.description).toContain("uncommitted");
    });

    it("does not offer the button while rows are still being classified", () => {
      state.classified = threeSafe();
      state.classifying = true;
      const r = render(<WorktreesPage />);
      expect(screen.queryByRole("button", { name: /remove 2 safe/i })).toBeNull();
      r.unmount();
      state.classifying = false;
    });

    // One safe row is a single click already; a bulk affordance for it
    // is noise.
    it("does not offer the button for a single safe worktree", () => {
      state.classified = [wt({ safety: { kind: "safe" } })];
      render(<WorktreesPage />);
      expect(screen.queryByRole("button", { name: /safe worktree/i })).toBeNull();
    });
  });

  // Coming back from a "safe to discard" verdict, the app's answer and
  // the user's disagreed and the app won -- with no way to act and no
  // way to find the row again among 124 candidates.
  describe("after an assessment", () => {
    // #701: a scratch branch reported "never pushed — commits exist
    // only here" beside "0 commits ahead". Both cannot be true, and the
    // user believed the alarming one and spent a session disproving it
    // by hand. The row now says which it is.
    it("says an empty branch has nothing to lose rather than claiming commits", () => {
      state.classified = [wt({ path: "/code/scratch", safety: { kind: "empty" } })];
      render(<WorktreesPage />);
      expect(screen.getByText(/no commits of its own/i)).toBeTruthy();
      expect(screen.queryByText(/only here/i)).toBeNull();
    });

    // The gate does NOT move. #701 is a report about the reporting,
    // and widening the app's only unrecoverable action is a separate
    // decision on separate evidence.
    //
    // The row is one-action, so "not removable" shows as Claudify
    // rather than as a greyed-out Remove -- the same shape every other
    // un-removable state gets. The plain Remove button must be absent:
    // if `Empty` had been folded into `Safe`, it would appear here,
    // enabled, and this is where that would be caught.
    it("still refuses one-click removal of an empty branch", () => {
      state.classified = [wt({ path: "/code/scratch", safety: { kind: "empty" } })];
      render(<WorktreesPage />);
      expect(screen.queryByRole("button", { name: "Remove" })).toBeNull();
      expect(screen.getByRole("button", { name: /claudify/i })).toBeTruthy();
    });

    // ...and the confirmation, which is the moment the decision is
    // made, must not repeat the false claim either.
    it("does not warn about unpushed commits when confirming an empty branch", () => {
      state.classified = [wt({ path: "/code/scratch", safety: { kind: "empty" } })];
      state.assessed = ["/code/scratch"];
      render(<WorktreesPage />);
      fireEvent.click(screen.getByRole("button", { name: /remove anyway/i }));
      expect(screen.getByText(/nothing on it would be lost/i)).toBeTruthy();
      expect(screen.queryByText(/not pushed anywhere/i)).toBeNull();
    });

    it("offers no override on a worktree that was never assessed", () => {
      state.classified = [wt({ path: "/code/a", safety: { kind: "never_pushed" } })];
      render(<WorktreesPage />);
      expect(screen.queryByRole("button", { name: /remove anyway/i })).toBeNull();
      expect(screen.getByRole("button", { name: /claudify/i })).toBeTruthy();
    });

    it("offers the override once that worktree has been assessed", () => {
      state.classified = [wt({ path: "/code/a", safety: { kind: "never_pushed" } })];
      state.assessed = ["/code/a"];
      render(<WorktreesPage />);
      expect(screen.getByRole("button", { name: /remove anyway/i })).toBeTruthy();
    });

    // Finding the row you just assessed is the part that made the
    // feature feel unfinished.
    it("sorts assessed rows to the top", () => {
      state.classified = [
        wt({ path: "/code/big", safety: { kind: "never_pushed" }, size_bytes: 9_000_000 }),
        wt({ path: "/code/assessed", safety: { kind: "never_pushed" }, size_bytes: 1 }),
      ];
      state.assessed = ["/code/assessed"];
      const { container } = render(<WorktreesPage />);
      // The cell carries the directory name and the branch, so match the
      // prefix rather than the whole string.
      const names = [...container.querySelectorAll(".font-mono")].map((n) => n.textContent);
      expect(names[0]).toMatch(/^assessed/);
    });

    // "Are you sure?" is not something anyone can act on. This is the
    // only genuinely unrecoverable action in the app.
    it("names the specific loss before removing", () => {
      state.classified = [wt({ path: "/code/a", safety: { kind: "never_pushed" } })];
      state.assessed = ["/code/a"];
      render(<WorktreesPage />);
      fireEvent.click(screen.getByRole("button", { name: /remove anyway/i }));

      const dialog = screen.getByRole("dialog");
      expect(within(dialog).getByText("/code/a")).toBeTruthy();
      expect(within(dialog).getByText(/not pushed anywhere/i)).toBeTruthy();
      expect(forceFn).not.toHaveBeenCalled();
    });

    it("removes only after the explicit confirmation", async () => {
      state.classified = [wt({ path: "/code/a", safety: { kind: "never_pushed" } })];
      state.assessed = ["/code/a"];
      render(<WorktreesPage />);
      fireEvent.click(screen.getByRole("button", { name: /remove anyway/i }));
      fireEvent.click(
        within(screen.getByRole("dialog")).getByRole("button", { name: /i have reviewed this/i }),
      );
      await waitFor(() => expect(forceFn).toHaveBeenCalledWith("/code/proj", "/code/a"));
    });

    // The bulk path is for the provably-safe set only, regardless of
    // what any assessment said.
    it("never includes an assessed-but-unsafe worktree in a bulk removal", () => {
      state.classified = [
        wt({ path: "/code/a", safety: { kind: "never_pushed" } }),
        wt({ path: "/code/b", safety: { kind: "safe" } }),
        wt({ path: "/code/c", safety: { kind: "safe" } }),
      ];
      state.assessed = ["/code/a"];
      render(<WorktreesPage />);
      fireEvent.click(screen.getByRole("button", { name: /remove 2 safe worktrees/i }));
      const dialog = screen.getByRole("dialog");
      expect(within(dialog).queryByText("/code/a")).toBeNull();
    });
  });

  /// "All repositories" fell through to `repos?.[0]` -- the FIRST repo,
  /// which `sort_for_sidebar` makes the largest. So across 37 repos the
  /// one question the view could not answer was the one needing every
  /// repo at once.
  describe("all repositories", () => {
    const twoRepos = [
      { identity: null, name: "proj-a", path: "/code/a", worktrees: [wt({ path: "/w/a", size_bytes: 10 })] },
      { identity: null, name: "proj-b", path: "/code/b", worktrees: [wt({ path: "/w/b", size_bytes: 900 })] },
    ];
    const showAll = () => {
      state.repos = twoRepos;
      useFilters.setState({
        filtersByView: { ...EMPTY, worktrees: {},
  branches: {} },
        view: "worktrees",
        panel: "list",
      });
      return render(<WorktreesPage />);
    };

    it("lists worktrees from every repository, not just the first", () => {
      showAll();
      expect(screen.getByText("proj-a")).toBeTruthy();
      expect(screen.getByText("proj-b")).toBeTruthy();
    });

    it("says how many repositories it spanned", () => {
      showAll();
      expect(screen.getByText(/2 worktrees across 2 repositories/i)).toBeTruthy();
    });

    it("puts the largest worktree first", () => {
      showAll();
      const names = screen.getAllByText(/^proj-[ab]$/).map((el) => el.textContent);
      expect(names[0]).toBe("proj-b");
    });

    // Acting needs a safety verdict, and classification is per repo at
    // ~16s across all of them -- so a row here navigates instead.
    it("opens a repository rather than offering to remove from here", () => {
      showAll();
      expect(screen.queryByRole("button", { name: /^remove$/i })).toBeNull();
      fireEvent.click(screen.getByText("proj-b"));
      expect(useFilters.getState().filtersByView.worktrees.repo).toBe("/code/b");
    });

    // A total that counts unmeasured sizes as zero is a confident wrong
    // answer, so it is labelled while any are still missing.
    it("calls the total partial while a size is unmeasured", () => {
      state.repos = [
        { identity: null, name: "a", path: "/code/a", worktrees: [wt({ path: "/w/a", size_bytes: null })] },
      ];
      useFilters.setState({
        filtersByView: { ...EMPTY, worktrees: {},
  branches: {} },
        view: "worktrees",
        panel: "list",
      });
      render(<WorktreesPage />);
      expect(screen.getByText(/at least/i)).toBeTruthy();
    });
  });

  /// The all-repositories rollup showed a total; the per-repo page --
  /// the one you land on after choosing a repo -- did not, so it could
  /// not answer "how much is this one holding?".
  describe("total size", () => {
    const withSizes = (sizes: (number | null)[]) => {
      state.repos = [
        {
          identity: null,
          name: "proj",
          path: "/code/proj",
          worktrees: sizes.map((size_bytes, i) =>
            wt({ path: `/w/${i}`, branch: `b-${i}`, size_bytes }),
          ),
        },
      ];
      useFilters.setState({
        filtersByView: { ...EMPTY, worktrees: { repo: "/code/proj" } },
        view: "worktrees",
        panel: "list",
      });
      return render(<WorktreesPage />);
    };

    it("sums the sizes it has", () => {
      withSizes([1024, 2048]);
      expect(screen.getByText(/3\.0 KB total|3 KB total/i)).toBeTruthy();
    });

    // Counting an unmeasured size as zero would report a confident
    // wrong number, so the total says so instead.
    it("says the total is partial while a size is missing", () => {
      withSizes([1024, null]);
      expect(screen.getByText(/at least/i)).toBeTruthy();
    });

    it("drops the qualifier once everything is measured", () => {
      withSizes([1024, 2048]);
      expect(screen.queryByText(/at least/i)).toBeNull();
    });

    // Nothing measured yet is not "0 bytes" -- it is no answer at all.
    it("shows no total before any size has arrived", () => {
      withSizes([null, null]);
      expect(screen.queryByText(/total/i)).toBeNull();
    });
  });

  /// #770: removal past the safety gate was reachable ONLY from the
  /// Claudify toast's "I read the assessment" button. A toast is for
  /// something you can ignore -- it leaves on a timer or on a stray
  /// click -- so the more careful the user was being, the more likely
  /// they lost the only route to the thing they had just asked for, and
  /// the only way back was another agent invocation.
  ///
  /// Every test here that asserts the kebab OFFERS removal also asserts
  /// the gate still refuses what it should. Removal is the one
  /// unrecoverable action in this app, and a menu that quietly widened
  /// it would be a far worse bug than the one being fixed.
  describe("the row kebab", () => {
    const openKebab = () =>
      fireEvent.click(screen.getByRole("button", { name: /more actions/i }));

    /// The affordance that used to live only on a toast, now on a
    /// surface that does not disappear.
    it("offers removal on a row whose gate refuses it, without an assessment", () => {
      state.classified = [wt({ safety: { kind: "never_pushed" } })];
      state.assessed = [];
      render(<WorktreesPage />);

      // The primary button is Claudify -- the gate has NOT moved.
      expect(screen.getByRole("button", { name: /claudify/i })).toBeTruthy();
      expect(screen.queryByRole("button", { name: "Remove" })).toBeNull();
      // ...and removal is reachable anyway, from the persistent menu.
      openKebab();
      expect(screen.getByRole("menuitem", { name: /remove worktree/i })).toBeTruthy();
    });

    /// Safe and MergedUpstreamDeleted are offered PLAINLY: both mean
    /// the work is on the default branch and the tree is clean, so both
    /// take the ordinary confirmation rather than the override.
    it.each([["safe"], ["merged_upstream_deleted"]] as const)(
      "sends a %s row to the plain confirmation",
      (kind) => {
        state.classified = [wt({ safety: { kind } })];
        render(<WorktreesPage />);
        openKebab();
        fireEvent.click(screen.getByRole("menuitem", { name: /remove worktree/i }));

        const dialog = screen.getByRole("dialog");
        expect(within(dialog).getByText(/remove this worktree\?/i)).toBeTruthy();
        // The ordinary dialog, not the override: no "I have reviewed
        // this", and no warning about an unrecoverable loss.
        expect(within(dialog).queryByText(/i have reviewed this/i)).toBeNull();
      },
    );

    /// A gate that a menu can walk past is not a gate. An unsafe row
    /// reaches removal only through the confirmation that names the
    /// specific loss -- the same one "Remove anyway…" opens.
    it("sends an unsafe row through the override confirmation, not the plain one", () => {
      state.classified = [wt({ safety: { kind: "never_pushed" } })];
      render(<WorktreesPage />);
      openKebab();
      fireEvent.click(screen.getByRole("menuitem", { name: /remove worktree/i }));

      const dialog = screen.getByRole("dialog");
      expect(within(dialog).getByText(/not pushed anywhere/i)).toBeTruthy();
      expect(within(dialog).getByRole("button", { name: /i have reviewed this/i })).toBeTruthy();
    });

    /// Nothing is deleted by opening a menu. The confirmation is the
    /// gate, and it has to be answered.
    it("removes nothing until the confirmation is answered", () => {
      forceFn.mockClear();
      state.classified = [wt({ safety: { kind: "never_pushed" } })];
      render(<WorktreesPage />);
      openKebab();
      fireEvent.click(screen.getByRole("menuitem", { name: /remove worktree/i }));
      expect(forceFn).not.toHaveBeenCalled();

      fireEvent.click(screen.getByRole("button", { name: /i have reviewed this/i }));
      expect(forceFn).toHaveBeenCalled();
    });

    /// #753's finding, reused rather than reworded.
    ///
    /// `remove_worktree_forced` relaxes Headstate's gate but still
    /// calls git WITHOUT `--force`, and git refuses a locked tree on
    /// its own account -- so a user who confirms here gets an error.
    /// The menu says so BEFORE the click, and says it in `forceWarning`'s
    /// words: two copies of a warning about an unrecoverable action are
    /// two chances to drift.
    it("warns on a locked row that forcing will not help", () => {
      state.classified = [
        wt({ safety: { kind: "locked", detail: lockOf() } }),
      ];
      render(<WorktreesPage />);
      openKebab();

      const item = screen.getByRole("menuitem", { name: /remove worktree/i });
      expect(item.textContent).toMatch(/git will refuse to remove it until it is unlocked/i);
      // Offered, not hidden: the user may well want to clear the lock,
      // and an absent item teaches nothing.
      expect(item).toBeTruthy();
    });

    /// #779 asserted the OPPOSITE of this, and deliberately: unlocking
    /// was #775's territory and was not being decided there. #775
    /// decided it, so the assertion is inverted rather than deleted --
    /// the old one is now the bug.
    ///
    /// What changed the balance is measurement, not taste: 20 of 44
    /// worktrees on the reporting machine are locked, all naming one
    /// pid that is alive only because it is the parent session, with
    /// nothing running in any of the directories. At 45% of the list,
    /// withholding the remedy leaves a view that cannot be used.
    it("offers an unlock action on a locked row", () => {
      state.classified = [
        wt({ safety: { kind: "locked", detail: lockOf() } }),
      ];
      render(<WorktreesPage />);
      openKebab();
      // Anchored, because the Remove item's own warning necessarily
      // contains the word "unlocked" -- a loose /unlock/ would match
      // the copy explaining that forcing will not help.
      const items = screen
        .getAllByRole("menuitem")
        .map((el) => el.textContent ?? "");
      expect(items.some((t) => /^\s*unlock/i.test(t))).toBe(true);
    });

    /// Only on a locked row. An unlock item beside an ordinary worktree
    /// would be an action with nothing to act on, and on this page a
    /// menu full of inapplicable verbs is how a user stops reading them.
    it("offers no unlock action on a row that is not locked", () => {
      state.classified = [wt({ safety: { kind: "unmerged" } })];
      render(<WorktreesPage />);
      openKebab();
      const items = screen
        .getAllByRole("menuitem")
        .map((el) => el.textContent ?? "");
      expect(items.some((t) => /^\s*unlock/i.test(t))).toBe(false);
    });

    /// The confirmation #753 asked for before any unlock button could
    /// exist: it must NAME the holder and the age, so clearing a claim
    /// cannot happen without reading it.
    it("names the holder and the age before unlocking", () => {
      unlockFn.mockClear();
      state.classified = [
        wt({ safety: { kind: "locked", detail: lockOf({ age_days: 5 }) } }),
      ];
      render(<WorktreesPage />);
      openKebab();
      fireEvent.click(screen.getByRole("menuitem", { name: /^unlock/i }));

      // Nothing is cleared by opening the dialog.
      expect(unlockFn).not.toHaveBeenCalled();

      const dialog = screen.getByRole("dialog");
      expect(dialog.textContent).toMatch(/5 days ago/);
      expect(dialog.textContent).toMatch(/some tool \(pid 123\)/);
    });

    /// The pid is the thing #775 says stops being presented as
    /// evidence. It is still SHOWN -- it is the locker's own words --
    /// but a live process must never be offered as proof the lock is
    /// current, because on the reporting machine that is true of all 20
    /// abandoned locks.
    it("does not present a running holder as proof the lock is live", () => {
      state.classified = [
        wt({
          safety: { kind: "locked", detail: lockOf({ holder_running: true }) },
        }),
      ];
      render(<WorktreesPage />);
      openKebab();
      fireEvent.click(screen.getByRole("menuitem", { name: /^unlock/i }));
      expect(screen.getByRole("dialog").textContent).toMatch(/weak evidence/i);
    });

    /// What is underneath, in the dialog as well as the row: it is the
    /// fact that turns unlocking from a leap into a decision, and the
    /// dialog is the moment the decision is actually made.
    it("says what is underneath the lock before clearing it", () => {
      state.classified = [
        wt({
          safety: {
            kind: "locked",
            detail: lockOf({ underlying: { kind: "safe" } }),
          },
        }),
      ];
      render(<WorktreesPage />);
      openKebab();
      fireEvent.click(screen.getByRole("menuitem", { name: /^unlock/i }));
      expect(screen.getByRole("dialog").textContent).toMatch(
        /merged, pushed|safe to delete/i,
      );
    });

    /// Unlocking must not be a quiet second route to removal. The
    /// confirmation clears the lock and nothing else.
    it("clears the lock and removes nothing when confirmed", async () => {
      unlockFn.mockClear();
      removeFn.mockClear();
      forceFn.mockClear();
      state.classified = [
        wt({ path: "/code/a", safety: { kind: "locked", detail: lockOf() } }),
      ];
      render(<WorktreesPage />);
      openKebab();
      fireEvent.click(screen.getByRole("menuitem", { name: /^unlock/i }));
      fireEvent.click(screen.getByRole("button", { name: /unlock it/i }));

      await waitFor(() =>
        expect(unlockFn).toHaveBeenCalledWith("/code/proj", "/code/a"),
      );
      expect(removeFn).not.toHaveBeenCalled();
      expect(forceFn).not.toHaveBeenCalled();
    });

    /// The main checkout is never a removal candidate, and a menu item
    /// offering to delete the repository's own checkout would be the
    /// worst possible thing to get wrong here.
    it("never offers to remove the main checkout", () => {
      state.classified = [
        wt({ path: "/code/proj", is_main: true, safety: { kind: "main_checkout" } }),
      ];
      render(<WorktreesPage />);
      openKebab();
      expect(screen.queryByRole("menuitem", { name: /remove worktree/i })).toBeNull();
    });

    /// An orphan has no repository for git to run in, so it is removed
    /// by a different call entirely -- and the row's own Delete button
    /// already offers it. A second, wrong route from the menu would be
    /// worse than none.
    it("leaves an orphan to its own Delete button", () => {
      state.classified = [wt({ safety: { kind: "orphaned" } })];
      render(<WorktreesPage />);
      expect(screen.getByRole("button", { name: /^delete$/i })).toBeTruthy();
      openKebab();
      expect(screen.queryByRole("menuitem", { name: /remove worktree/i })).toBeNull();
    });

    /// A prunable row no longer offers the WRONG verb, and is not left a
    /// dead end either (#793).
    ///
    /// Both halves in one test, because either alone is an incomplete
    /// fix. The menu used to offer "Remove worktree" on a row whose
    /// directory is gone, routed at `remove_worktree_forced` -- which
    /// runs `git worktree remove` against nothing. Deleting that item
    /// without naming the action that does work would only make the dead
    /// end quieter.
    it("points a prunable row at the prune action instead of offering removal", () => {
      state.classified = [
        wt({ safety: { kind: "prunable", detail: "gitdir file points to non-existent location" } }),
      ];
      render(<WorktreesPage />);
      openKebab();
      expect(screen.queryByRole("menuitem", { name: /remove worktree/i })).toBeNull();
      expect(screen.getByRole("menu").textContent).toMatch(/prune stale registrations/i);
    });
  });

  /// #819: four detached worktrees read "could not determine: detached
  /// HEAD" while git could answer their merge status instantly. A row
  /// that says "I cannot tell you anything" is the dead end the issue
  /// reports, and the acceptance criterion is that such a row gets a real
  /// action.
  describe("merged detached worktrees", () => {
    const detachedMergedWt = () =>
      wt({
        path: "/code/enc-ui-aws-city-asn",
        // Empty, which is the whole point: there is no branch, and the
        // row must still be classifiable and actionable.
        branch: "",
        safety: { kind: "detached_merged", detail: "detached at v1.13.0~26" },
      });

    /// The action, asserted where the user looks for it. Before #819 this
    /// row was `unknown`: a grey verdict, a disabled Remove, and no
    /// Claudify -- the kebab's Claudify was gated on `assessed`, which
    /// only the Claudify toast sets, which could not be reached. A closed
    /// loop, broken here by the row being genuinely removable.
    it("counts a merged detached worktree as safe and offers Remove", () => {
      state.classified = [detachedMergedWt()];
      render(<WorktreesPage />);
      expect(screen.getByText(/1 safe to remove/i)).toBeTruthy();
      const remove = screen.getByRole("button", { name: /^remove$/i });
      expect(remove.hasAttribute("disabled")).toBe(false);
    });

    /// The prose, in the order #819 asks for: the answer, then what the
    /// sha is, then the reassurance. "detached at v1.13.0~26" is what
    /// makes the row identifiable -- `git name-rev` resolved all four of
    /// the reported worktrees to a tag in milliseconds, and "detached"
    /// alone says only what the checkout lacks.
    it("leads with merged and names what the sha resolves to", () => {
      state.classified = [detachedMergedWt()];
      render(<WorktreesPage />);
      const verdict = screen.getByText(/^merged —/);
      expect(verdict.textContent).toContain("v1.13.0~26");
      expect(verdict.textContent).toContain("no branch to delete");
      expect(verdict.className).toContain("3fb950");
    });

    /// The #776 property, asserted at the UI boundary too.
    ///
    /// That fix exists because detached rows used to report
    /// `NeverPushed` -- "commits exist only here", the app's strongest
    /// refusal -- over checkouts whose commits were on the default branch
    /// and on the remote, measured at 12 of 43 real rows. #819 answers the
    /// merge question for these rows and must not reintroduce any claim
    /// about push state, which genuinely needs a branch.
    it("never claims a branchless checkout holds unique commits", () => {
      state.classified = [detachedMergedWt()];
      render(<WorktreesPage />);
      expect(screen.queryByText(/only here/i)).toBeNull();
      expect(screen.queryByText(/could not determine/i)).toBeNull();
    });

    /// An UNMERGED detached checkout stays unknown and stays refused.
    ///
    /// Without this the feature would be indistinguishable from "call
    /// every detached row safe", which is the opposite of what #819 asks:
    /// it asks that "unknown" be said only about what is genuinely
    /// unknown. A commit that exists nowhere but this directory is
    /// exactly that.
    it("still refuses a detached checkout that is not on the default branch", () => {
      state.classified = [
        wt({
          path: "/code/scratch",
          branch: "",
          safety: {
            kind: "unknown",
            detail: "detached HEAD at v1.13.0~26 — not found on main",
          },
        }),
      ];
      render(<WorktreesPage />);
      expect(screen.getByText(/0 safe to remove/i)).toBeTruthy();
      expect(screen.getByRole("button", { name: /^remove$/i }).hasAttribute("disabled")).toBe(true);
    });
  });

  /// #793: the app diagnosed prunable worktrees, named `git worktree
  /// prune` in its own confirmation copy, and never ran it anywhere.
  describe("stale registrations", () => {
    const prunableWt = (path: string) =>
      wt({
        path,
        branch: "",
        safety: { kind: "prunable", detail: "gitdir file points to non-existent location" },
      });

    beforeEach(() => {
      pruneFn.mockClear();
      pruneFn.mockResolvedValue(2);
    });

    /// Counted in their OWN words, never folded into the green count.
    /// A prunable worktree is not disk to reclaim -- the directory is
    /// already gone -- so reporting it as "safe to remove" would claim
    /// recoverable space that does not exist.
    it("counts stale registrations separately from safe ones", () => {
      state.classified = [
        wt({ path: "/code/a", safety: { kind: "safe" } }),
        prunableWt("/code/b"),
        prunableWt("/code/c"),
      ];
      render(<WorktreesPage />);
      expect(screen.getByText(/1 safe to remove/i)).toBeTruthy();
      // The exact string, anchored: the Prune BUTTON also says "2 stale
      // registrations", and this assertion is about the count being its
      // own fact on the line rather than only a label on an action.
      expect(screen.getByText("2 stale registrations")).toBeTruthy();
    });

    /// One affordance over the repository, because `git worktree prune`
    /// takes no path. A per-row button would clear all of them and
    /// appear to clear one.
    it("runs git worktree prune for the whole repository", async () => {
      state.classified = [prunableWt("/code/b"), prunableWt("/code/c")];
      render(<WorktreesPage />);
      fireEvent.click(screen.getByRole("button", { name: /prune 2 stale registrations/i }));
      await waitFor(() => expect(pruneFn).toHaveBeenCalledWith("/code/proj"));
      // No confirmation dialog: nothing recoverable is deleted, and a
      // dialog over an action with no loss teaches users to click
      // through the ones that matter.
      expect(screen.queryByRole("dialog")).toBeNull();
    });

    /// The COUNT is the result. `git worktree prune` is silent on
    /// success, so a bare "Pruned" could not tell a cleared repository
    /// from one somebody had already pruned in a terminal.
    it("reports how many registrations went", async () => {
      state.classified = [prunableWt("/code/b"), prunableWt("/code/c")];
      render(<WorktreesPage />);
      fireEvent.click(screen.getByRole("button", { name: /prune 2 stale/i }));
      await waitFor(() =>
        expect(toastSuccess).toHaveBeenCalledWith(
          expect.stringMatching(/pruned 2 stale registrations/i),
          expect.anything(),
        ),
      );
    });

    /// Zero is a legitimate answer and not a success. The scan is a
    /// snapshot, so a second click or a terminal prune in between leaves
    /// nothing to do -- and "Pruned 0" dressed as a success would read
    /// as a result that did not happen.
    it("says plainly when there was nothing to prune", async () => {
      pruneFn.mockResolvedValue(0);
      state.classified = [prunableWt("/code/b")];
      render(<WorktreesPage />);
      fireEvent.click(screen.getByRole("button", { name: /prune 1 stale/i }));
      await waitFor(() => expect(toastInfo).toHaveBeenCalled());
      expect(toastSuccess).not.toHaveBeenCalled();
    });

    /// Silent when there is nothing stale. Most repositories have none,
    /// and a permanent "0 stale registrations" with a disabled button
    /// beside it is furniture.
    it("offers nothing when no registration is stale", () => {
      state.classified = [wt({ safety: { kind: "safe" } })];
      render(<WorktreesPage />);
      expect(screen.queryByText(/stale registration/i)).toBeNull();
      expect(screen.queryByRole("button", { name: /prune/i })).toBeNull();
    });
  });

  /// #792: `holder_running` was computed on every scan and read only by
  /// the unlock dialog, so the user learned the holder was dead after
  /// deciding to unlock.
  describe("locks with no live holder", () => {
    const deadLockWt = (path: string) =>
      wt({
        path,
        safety: {
          kind: "locked",
          detail: lockOf({ holder_running: false, age_days: 2 }),
        },
      });

    beforeEach(() => {
      unlockManyFn.mockClear();
      unlockManyFn.mockImplementation((_r, paths) =>
        Promise.resolve(paths.map((p) => ({ path: p, error: null }))),
      );
    });

    /// On the ROW, which is where the decision is made -- not only in a
    /// dialog the user opens after deciding. And AFTER git's own reason,
    /// never instead of it: the reason is the locker's own words.
    it("says on the row that the holder process is gone", () => {
      state.classified = [deadLockWt("/code/a")];
      render(<WorktreesPage />);
      const line = screen.getByText(/holder process is gone/i).textContent ?? "";
      expect(line).toContain("some tool (pid 123)");
      expect(line.indexOf("some tool")).toBeLessThan(line.indexOf("holder process"));
    });

    /// Silent about a holder that is running, or one the reason named
    /// nobody to check. A live pid was true for all 20 locks on the
    /// reporting machine and every one was abandoned, so announcing it
    /// would spend weak evidence as proof; `null` means the question was
    /// never asked, which must not read as a negative answer.
    it("says nothing about a holder it could not check or that is running", () => {
      state.classified = [
        wt({ path: "/code/a", safety: { kind: "locked", detail: lockOf({ holder_running: true }) } }),
        wt({ path: "/code/b", safety: { kind: "locked", detail: lockOf({ holder_running: null }) } }),
      ];
      render(<WorktreesPage />);
      expect(screen.queryByText(/holder process is gone/i)).toBeNull();
    });

    /// A bulk affordance, because the condition is bulk: five after one
    /// reboot on the reporting machine, 20+ historically. Five dialogs
    /// each saying "the process it names is no longer running" add
    /// clicks, not judgement.
    it("unlocks every provably-dead lock in one gesture", async () => {
      state.classified = [deadLockWt("/code/a"), deadLockWt("/code/b")];
      render(<WorktreesPage />);
      fireEvent.click(screen.getByRole("button", { name: /unlock 2 abandoned locks/i }));
      // Still behind a dialog, unlike prune: this clears a claim on a
      // directory that is still there, and the scope is worth reviewing
      // once.
      expect(screen.getByRole("dialog").textContent).toContain("/code/a");
      fireEvent.click(screen.getByRole("button", { name: /^unlock 2 locks$/i }));
      await waitFor(() =>
        expect(unlockManyFn).toHaveBeenCalledWith("/code/proj", ["/code/a", "/code/b"]),
      );
    });

    /// The narrowing that keeps #753's refusal intact. A lock whose
    /// holder is alive, or whose reason named nobody to check, is not in
    /// the batch: a bulk action over claims that MIGHT be live is
    /// exactly what that issue declined to offer.
    it("leaves locks whose holder might be live out of the batch", async () => {
      state.classified = [
        deadLockWt("/code/a"),
        wt({ path: "/code/b", safety: { kind: "locked", detail: lockOf({ holder_running: true }) } }),
        wt({ path: "/code/c", safety: { kind: "locked", detail: lockOf({ holder_running: null }) } }),
      ];
      render(<WorktreesPage />);
      fireEvent.click(screen.getByRole("button", { name: /unlock 1 abandoned lock$/i }));
      fireEvent.click(screen.getByRole("button", { name: /^unlock 1 lock$/i }));
      await waitFor(() =>
        expect(unlockManyFn).toHaveBeenCalledWith("/code/proj", ["/code/a"]),
      );
    });

    /// A partial failure is reported, never swallowed. The scan is a
    /// snapshot, so a row somebody else unlocked in between is an
    /// ordinary race -- and a bare "Unlocked 2" over one refusal would
    /// misreport which locks are still in place.
    it("names the locks that could not be cleared", async () => {
      unlockManyFn.mockResolvedValue([
        { path: "/code/a", error: null },
        { path: "/code/b", error: "that worktree is not locked" },
      ]);
      state.classified = [deadLockWt("/code/a"), deadLockWt("/code/b")];
      render(<WorktreesPage />);
      fireEvent.click(screen.getByRole("button", { name: /unlock 2 abandoned locks/i }));
      fireEvent.click(screen.getByRole("button", { name: /^unlock 2 locks$/i }));
      await waitFor(() =>
        expect(toastError).toHaveBeenCalledWith(
          expect.stringMatching(/1 of 2 could not be unlocked/i),
          expect.objectContaining({ description: "b: that worktree is not locked" }),
        ),
      );
      expect(toastSuccess).not.toHaveBeenCalled();
    });

    /// Removal does not become easier. `is_safe()` still excludes
    /// `locked`, so the row's own button stays disabled -- the lock
    /// becomes EASY TO CLEAR, not silently removable.
    it("does not make a dead-holder lock removable", () => {
      state.classified = [deadLockWt("/code/a")];
      render(<WorktreesPage />);
      expect(screen.getByRole("button", { name: /^remove$/i })).toHaveProperty("disabled", true);
    });

    /// Offered only when there is something to offer.
    it("offers nothing when every lock has a live or unknown holder", () => {
      state.classified = [
        wt({ safety: { kind: "locked", detail: lockOf({ holder_running: true }) } }),
      ];
      render(<WorktreesPage />);
      expect(screen.queryByRole("button", { name: /abandoned lock/i })).toBeNull();
    });
  });

  /// #771: the list rendered name, size and age on every row and could
  /// order by none of them, on a page whose whole purpose is "which of
  /// these is biggest".
  describe("sorting", () => {
    const shownNames = (container: HTMLElement) =>
      [...container.querySelectorAll(".font-mono")].map((n) => n.textContent ?? "");

    const threeRows = () => {
      state.classified = [
        wt({ path: "/code/bravo", size_bytes: 500, last_commit: "2026-01-01T00:00:00Z" }),
        wt({ path: "/code/alpha", size_bytes: 9_000, last_commit: "2025-01-01T00:00:00Z" }),
        wt({ path: "/code/charlie", size_bytes: 50, last_commit: "2026-09-01T00:00:00Z" }),
      ];
    };

    it("defaults to largest first, which is the question the page exists for", () => {
      threeRows();
      const { container } = render(<WorktreesPage />);
      expect(shownNames(container)[0]).toMatch(/^alpha/);
    });

    it("orders by name, size and age, in both directions", () => {
      threeRows();
      const { container } = render(<WorktreesPage />);
      const select = screen.getByRole("combobox", { name: /sort worktrees/i });

      const first = (value: string) => {
        fireEvent.change(select, { target: { value } });
        return shownNames(container)[0];
      };

      expect(first("size-desc")).toMatch(/^alpha/);
      expect(first("size-asc")).toMatch(/^charlie/);
      // Least recently committed first: the safe wins.
      expect(first("age-desc")).toMatch(/^alpha/);
      expect(first("age-asc")).toMatch(/^charlie/);
      expect(first("name-asc")).toMatch(/^alpha/);
      expect(first("name-desc")).toMatch(/^charlie/);
    });

    /// The streaming answer, and the reason the order is a snapshot.
    ///
    /// A size landing must NOT re-order the list under the cursor: the
    /// button it would move out from under you removes a directory. So
    /// the row's displayed size updates live while its POSITION does
    /// not, and the page offers an explicit gesture to apply the rest.
    it("does not re-order rows as sizes land", () => {
      state.classified = [
        wt({ path: "/code/measured", size_bytes: 500 }),
        wt({ path: "/code/pending", size_bytes: null }),
      ];
      state.sizing = true;
      const { container, rerender } = render(<WorktreesPage />);
      expect(shownNames(container)[0]).toMatch(/^measured/);

      // `pending` turns out to be far the bigger of the two...
      state.partialSizes = new Map([["/code/pending", 9_000_000]]);
      rerender(<WorktreesPage />);

      // ...its size is shown at once, because withholding a landed
      // measurement would be the other kind of dishonesty...
      expect(screen.getByText("8.6 MB")).toBeTruthy();
      // ...but the ORDER has not moved under the user.
      expect(shownNames(container)[0]).toMatch(/^measured/);
    });

    /// A frozen order that says nothing is merely stale. The page has
    /// to admit the list is out of date and offer the one click that
    /// fixes it, or the user reads a "Largest first" that silently is
    /// not.
    it("offers an explicit re-sort once newly measured rows exist", () => {
      state.classified = [
        wt({ path: "/code/measured", size_bytes: 500 }),
        wt({ path: "/code/pending", size_bytes: null }),
      ];
      state.sizing = true;
      const { container, rerender } = render(<WorktreesPage />);
      expect(screen.queryByRole("button", { name: /re-sort/i })).toBeNull();

      state.partialSizes = new Map([["/code/pending", 9_000_000]]);
      rerender(<WorktreesPage />);

      const resort = screen.getByRole("button", { name: /re-sort/i });
      // "out of date" rather than the original "newly measured": the
      // count grew a second source in #817 -- an assessment landing
      // re-ranks rows just as a measurement does -- so the label can no
      // longer name measurement as the only cause. The CONTRACT this
      // test exists for is unchanged and asserted either side of here:
      // the count is surfaced, and only an explicit click applies it.
      expect(resort.textContent).toMatch(/1 out of date/);
      fireEvent.click(resort);

      // The gesture applies the measurements that had landed since.
      expect(shownNames(container)[0]).toMatch(/^pending/);
      expect(screen.queryByRole("button", { name: /re-sort/i })).toBeNull();
    });

    /// Changing the sort is itself an explicit gesture, so it takes a
    /// fresh snapshot -- the user asked for a new order and gets one
    /// built from everything known right now.
    it("takes a fresh snapshot when the sort is changed", () => {
      state.classified = [
        wt({ path: "/code/measured", size_bytes: 500 }),
        wt({ path: "/code/pending", size_bytes: null }),
      ];
      state.sizing = true;
      const { container, rerender } = render(<WorktreesPage />);

      state.partialSizes = new Map([["/code/pending", 9_000_000]]);
      rerender(<WorktreesPage />);
      expect(shownNames(container)[0]).toMatch(/^measured/);

      fireEvent.change(screen.getByRole("combobox", { name: /sort worktrees/i }), {
        target: { value: "size-desc" },
      });
      expect(shownNames(container)[0]).toMatch(/^pending/);
    });

    /// Name is fully known at first render, so it is the escape hatch
    /// while sizes are still landing -- and it must never carry the
    /// "newly measured" nag, which would be asking the user to re-apply
    /// something that cannot change the order.
    it("does not nag about measurements on a name sort", () => {
      state.classified = [
        wt({ path: "/code/measured", size_bytes: 500 }),
        wt({ path: "/code/pending", size_bytes: null }),
      ];
      state.sizing = true;
      const { rerender } = render(<WorktreesPage />);
      fireEvent.change(screen.getByRole("combobox", { name: /sort worktrees/i }), {
        target: { value: "name-asc" },
      });

      state.partialSizes = new Map([["/code/pending", 9_000_000]]);
      rerender(<WorktreesPage />);
      expect(screen.queryByRole("button", { name: /re-sort/i })).toBeNull();
    });

    /// Arriving at the page is not a choice to protect (#817).
    ///
    /// The freeze above exists so a row cannot slide out from under a
    /// cursor reaching for an order the USER picked. On first paint
    /// nobody has picked anything -- the default is "largest first" and
    /// the first measurements land after the mount -- so the page used
    /// to greet an untouched arrival with "re-sort — N out of date",
    /// which reports that data arrived rather than that anything is
    /// wrong. The snapshot is re-taken once when the first pass settles,
    /// so the list is correctly ordered AND silent.
    it("does not prompt on first arrival, and is already correctly sorted", () => {
      state.classified = [
        wt({ path: "/code/small", size_bytes: 500 }),
        wt({ path: "/code/big", size_bytes: null }),
      ];
      // The initial burst, mid-flight: nothing has settled yet.
      state.sizing = true;
      const { container, rerender } = render(<WorktreesPage />);

      // `big` turns out to be the larger, arriving after the mount --
      // the ordinary case, not an edge one.
      state.partialSizes = new Map([["/code/big", 9_000_000]]);
      rerender(<WorktreesPage />);

      // ...and the burst settles.
      state.sizing = false;
      state.sizes = new Map([
        ["/code/small", 500],
        ["/code/big", 9_000_000],
      ]);
      rerender(<WorktreesPage />);

      // The order is right, without the user having done anything...
      expect(shownNames(container)[0]).toMatch(/^big/);
      // ...so there is nothing to prompt about.
      expect(screen.queryByRole("button", { name: /re-sort/i })).toBeNull();
    });

    /// The one free re-sort is spent ONCE, not on every pass (#817).
    ///
    /// The guard that matters: a clock that bumped whenever sizing went
    /// idle would re-order the list on each background refetch, which is
    /// the live re-ordering this whole design calls the one genuinely
    /// unsafe option. After the first settle the freeze is back in full
    /// force.
    it("re-freezes after the first pass settles", () => {
      state.classified = [
        wt({ path: "/code/small", size_bytes: 500 }),
        wt({ path: "/code/big", size_bytes: null }),
      ];
      state.sizing = true;
      const { container, rerender } = render(<WorktreesPage />);
      state.sizing = false;
      state.sizes = new Map([["/code/small", 500]]);
      rerender(<WorktreesPage />);

      // A later pass finds `big` is enormous. It must NOT move on its
      // own, however idle the query goes.
      state.sizing = true;
      rerender(<WorktreesPage />);
      state.sizing = false;
      state.sizes = new Map([
        ["/code/small", 500],
        ["/code/big", 9_000_000],
      ]);
      rerender(<WorktreesPage />);

      expect(shownNames(container)[0]).toMatch(/^small/);
      // The honest part: frozen, and saying so.
      expect(screen.getByRole("button", { name: /re-sort/i })).toBeTruthy();
    });

    /// Assessments re-rank rows too, and used to do it silently (#817).
    ///
    /// `sortWorktrees` puts assessed rows above unassessed ones before
    /// it looks at any axis, so marking one assessed invalidates the
    /// displayed order exactly as a measurement does. The count read
    /// only sizes, so this half of the data went stale with the page
    /// saying nothing -- and on a name sort, where sizes are ignored, it
    /// was the only thing that could go stale at all.
    it("counts an assessment landing, even on a name sort", () => {
      state.classified = [
        wt({ path: "/code/aaa", size_bytes: 500 }),
        wt({ path: "/code/zzz", size_bytes: 500 }),
      ];
      state.assessed = [];
      const { container, rerender } = render(<WorktreesPage />);
      fireEvent.change(screen.getByRole("combobox", { name: /sort worktrees/i }), {
        target: { value: "name-asc" },
      });
      expect(shownNames(container)[0]).toMatch(/^aaa/);
      expect(screen.queryByRole("button", { name: /re-sort/i })).toBeNull();

      // `zzz` is assessed, so it now outranks `aaa` -- but not under the
      // cursor.
      state.assessed = ["/code/zzz"];
      rerender(<WorktreesPage />);
      expect(shownNames(container)[0]).toMatch(/^aaa/);

      const resort = screen.getByRole("button", { name: /re-sort/i });
      expect(resort.textContent).toMatch(/1 out of date/);
      fireEvent.click(resort);
      expect(shownNames(container)[0]).toMatch(/^zzz/);
    });

    /// An appearing advisory control must not move a destructive one
    /// (#817).
    ///
    /// The re-sort button used to render immediately before the bulk
    /// "Remove N safe worktrees" button in one `flex flex-wrap` toolbar,
    /// so its arrival shoved a directory-deleting button sideways or
    /// onto a second line. It now lives in a group with the Sort select
    /// it modifies, which is both where it belongs and out of Remove's
    /// way. Asserted structurally, since jsdom lays nothing out: the two
    /// buttons have no common ancestor below the toolbar, so neither can
    /// be a sibling the other displaces.
    it("keeps the re-sort button out of the bulk Remove button's group", () => {
      state.classified = [
        wt({ path: "/code/measured", size_bytes: 500, safety: { kind: "safe" } }),
        wt({ path: "/code/pending", size_bytes: null, safety: { kind: "safe" } }),
      ];
      state.sizing = true;
      const { rerender } = render(<WorktreesPage />);
      state.partialSizes = new Map([["/code/pending", 9_000_000]]);
      rerender(<WorktreesPage />);

      const resort = screen.getByRole("button", { name: /re-sort/i });
      const remove = screen.getByRole("button", { name: /remove 2 safe worktrees/i });
      // The re-sort button is grouped with the Sort select...
      const group = resort.parentElement as HTMLElement;
      expect(group.querySelector("select[aria-label='Sort worktrees']")).toBeTruthy();
      // ...and the destructive button is outside that group entirely.
      expect(group.contains(remove)).toBe(false);
    });
  });
});
