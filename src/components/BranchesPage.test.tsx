import { QueryClient, QueryClientProvider, useQuery } from "@tanstack/react-query";
import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import type { Branch, Deletable } from "@/types/pr";

const listFn = vi.hoisted(() => vi.fn<(...a: unknown[]) => Promise<Branch[]>>());
const delLocal = vi.hoisted(() =>
  vi.fn<(...a: unknown[]) => Promise<{ name: string; error: string | null }[]>>(),
);
const delRemote = vi.hoisted(() =>
  vi.fn<(...a: unknown[]) => Promise<{ name: string; error: string | null }[]>>(),
);
const toasts = vi.hoisted(() => ({
  success: vi.fn(),
  error: vi.fn(),
  warning: vi.fn(),
  info: vi.fn(),
}));

vi.mock("sonner", () => ({ toast: toasts }));

/// What `useBranchScan` reports, per test. Set by `streamingScan`.
///
/// The real hook is exercised end to end in `hooks.branchScan.test.tsx`;
/// here it is a value, so a page test can pin the SHAPE the page is
/// asked to render — including the shape it must never be able to tell
/// apart from a finished one on its own.
const scanState = vi.hoisted(() => ({
  current: { branches: [] as Branch[], total: null as number | null, classified: 0 },
}));

// The hooks, not the whole tauri module: `api/hooks` pulls in the rest
// of the app's commands, and mocking that module wholesale would make
// this test depend on every one of them.
vi.mock("../api/hooks", () => ({
  useBranches: (repoPath: string | undefined) => {
    const q = useQuery({
      queryKey: ["branches", repoPath],
      queryFn: () => listFn(repoPath),
      enabled: !!repoPath,
      retry: false,
    });
    return q;
  },
  useBranchScan: () => scanState.current,
}));
vi.mock("../api/tauri", () => ({
  listBranches: listFn,
  deleteBranches: delLocal,
  deleteRemoteBranches: delRemote,
}));

import { BranchesPage, reason } from "./BranchesPage";
import { useFilters } from "../store/filters";

const branch = (over: Partial<Branch> = {}): Branch => ({
  name: "feature",
  location: "local",
  upstream: null,
  ahead: 0,
  behind: 0,
  committed: new Date().toISOString(),
  author: "octocat",
  tip: "abc1234",
  deletable: { kind: "merged", how: "squash" },
  ...over,
});

const show = () => {
  const qc = new QueryClient({ defaultOptions: { queries: { retry: false } } });
  return render(
    <QueryClientProvider client={qc}>
      <BranchesPage />
    </QueryClientProvider>,
  );
};

describe("reason", () => {
  /// Every case must produce words. A branch with no reason shown is
  /// the failure this whole tagged union exists to prevent.
  it("explains every state, including the ones that block deletion", () => {
    const cases: Deletable[] = [
      { kind: "merged", how: "ancestor" },
      { kind: "merged", how: "squash" },
      { kind: "defaultBranch" },
      { kind: "checkedOut", path: "/w/foo" },
      { kind: "unmerged", ahead: 3 },
      { kind: "pending" },
      { kind: "unknown", reason: "git failed" },
    ];
    for (const c of cases) expect(reason(c).length).toBeGreaterThan(0);
  });

  /// A squash merge is established by comparing content, not ancestry.
  /// The UI says which, because they do not deserve equal confidence.
  it("distinguishes a squash merge from an ancestor merge", () => {
    expect(reason({ kind: "merged", how: "squash" })).toMatch(/squash/i);
    expect(reason({ kind: "merged", how: "ancestor" })).not.toMatch(/squash/i);
  });

  it("counts commits when a branch is unmerged", () => {
    expect(reason({ kind: "unmerged", ahead: 1 })).toMatch(/1 commit\b/);
    expect(reason({ kind: "unmerged", ahead: 4 })).toMatch(/4 commits/);
  });

  it("names where a checked-out branch is checked out", () => {
    expect(reason({ kind: "checkedOut", path: "/w/foo" })).toContain("/w/foo");
  });
});

describe("BranchesPage", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    scanState.current = { branches: [], total: null, classified: 0 };
    useFilters.getState().setView("branches");
    useFilters.getState().setFilter("repo", "/code/app");
    listFn.mockResolvedValue([
      branch({ name: "done" }),
      branch({
        name: "wip",
        deletable: { kind: "unmerged", ahead: 2 },
      }),
    ]);
    delLocal.mockResolvedValue([{ name: "done", error: null }]);
    delRemote.mockResolvedValue([{ name: "origin/shipped", error: null }]);
  });

  it("lists branches with the reason each one is or is not deletable", async () => {
    show();
    expect(await screen.findByText("done")).toBeTruthy();
    expect(screen.getByText(/Merged \(squashed\)/)).toBeTruthy();
    expect(screen.getByText(/Not merged — 2 commits/)).toBeTruthy();
  });

  /// The gate the UI enforces before the backend re-checks it.
  it("does not let an unmerged branch be selected", async () => {
    show();
    await screen.findByText("done");
    expect(screen.getByLabelText("done").hasAttribute("disabled")).toBe(false);
    expect(screen.getByLabelText("wip").hasAttribute("disabled")).toBe(true);
  });

  it("deletes the selected local branches through the scope modal", async () => {
    show();
    await screen.findByText("done");
    fireEvent.click(screen.getByLabelText("done"));
    fireEvent.click(screen.getByRole("button", { name: /^delete 1…/i }));

    // The modal asks where, and a local-only selection has one answer.
    expect(await screen.findByText(/where should these be deleted/i)).toBeTruthy();
    fireEvent.click(screen.getByRole("button", { name: /delete 1 branch locally/i }));

    await waitFor(() => expect(delLocal).toHaveBeenCalledTimes(1));
    expect(delLocal).toHaveBeenCalledWith("/code/app", ["done"]);
    expect(delRemote).not.toHaveBeenCalled();
  });

  /// #472: one action for the whole merged set, selecting exactly what
  /// ticking each row by hand would select.
  it("selects every merged branch and no unmerged one", async () => {
    show();
    await screen.findByText("done");
    fireEvent.click(screen.getByRole("button", { name: /select all 1 merged/i }));

    expect((screen.getByLabelText("done") as HTMLInputElement).checked).toBe(true);
    expect((screen.getByLabelText("wip") as HTMLInputElement).checked).toBe(false);
  });

  /// A remote-only selection has no local side, so "locally" is not
  /// offered -- an option with nothing to act on is how a user clicks
  /// it and is told nothing happened.
  it("offers only the remote scope for a remote-only selection", async () => {
    listFn.mockResolvedValue([branch({ name: "origin/shipped", location: "remote" })]);
    show();
    await screen.findByText("origin/shipped");
    fireEvent.click(screen.getByLabelText("origin/shipped"));
    fireEvent.click(screen.getByRole("button", { name: /^delete 1…/i }));

    await screen.findByText(/where should these be deleted/i);
    expect(screen.getByText(/on the remote only/i)).toBeTruthy();
    expect(screen.queryByText(/^locally only$/i)).toBeNull();

    fireEvent.click(screen.getByRole("button", { name: /delete 1 branch on the remote/i }));
    await waitFor(() => expect(delRemote).toHaveBeenCalledTimes(1));
    expect(delRemote).toHaveBeenCalledWith("/code/app", ["origin/shipped"]);
    expect(delLocal).not.toHaveBeenCalled();
  });

  /// #473: a tracked branch exists in both places. Deleting it
  /// "locally" only used to be the ONLY thing offered, leaving the
  /// remote branch alive while the user believed it was cleaned up.
  it("deletes a tracked branch in both places when asked to", async () => {
    listFn.mockResolvedValue([
      branch({ name: "shipped", location: "tracked", upstream: "origin/shipped" }),
    ]);
    show();
    await screen.findByText("shipped");
    fireEvent.click(screen.getByLabelText("shipped"));
    fireEvent.click(screen.getByRole("button", { name: /^delete 1…/i }));

    await screen.findByText(/where should these be deleted/i);
    fireEvent.click(screen.getByText(/both here and on the remote/i));
    fireEvent.click(
      screen.getByRole("button", { name: /delete 1 branch locally and on the remote/i }),
    );

    await waitFor(() => expect(delLocal).toHaveBeenCalledTimes(1));
    expect(delLocal).toHaveBeenCalledWith("/code/app", ["shipped"]);
    // The UPSTREAM name, not the local one.
    expect(delRemote).toHaveBeenCalledWith("/code/app", ["origin/shipped"]);
  });

  /// #492: the two halves must not run concurrently.
  ///
  /// They did, and for a tracked branch both name the same branch -- so
  /// the local delete removed the ref while the remote half was still
  /// re-checking it, and the remote half reported "no longer exists"
  /// for a deletion that would have worked.
  ///
  /// Remote leads: it is the half nothing can undo, so if it fails the
  /// local ref is still there to retry from.
  it("deletes the remote before the local ref, not at the same time", async () => {
    listFn.mockResolvedValue([
      branch({ name: "shipped", location: "tracked", upstream: "origin/shipped" }),
    ]);
    const order: string[] = [];
    let releaseRemote!: (v: unknown[]) => void;
    delRemote.mockReturnValue(
      new Promise((resolve) => {
        order.push("remote:start");
        releaseRemote = resolve as (v: unknown[]) => void;
      }),
    );
    delLocal.mockImplementation(() => {
      order.push("local:start");
      return Promise.resolve([{ name: "shipped", error: null }]);
    });

    show();
    await screen.findByText("shipped");
    fireEvent.click(screen.getByLabelText("shipped"));
    fireEvent.click(screen.getByRole("button", { name: /^delete 1…/i }));
    await screen.findByText(/where should these be deleted/i);
    fireEvent.click(screen.getByText(/both here and on the remote/i));
    fireEvent.click(
      screen.getByRole("button", { name: /delete 1 branch locally and on the remote/i }),
    );

    // The remote call is in flight and the local one has NOT started.
    await waitFor(() => expect(delRemote).toHaveBeenCalledTimes(1));
    expect(delLocal).not.toHaveBeenCalled();

    releaseRemote([{ name: "origin/shipped", error: null }]);
    await waitFor(() => expect(delLocal).toHaveBeenCalledTimes(1));
    expect(order).toEqual(["remote:start", "local:start"]);
  });

  /// A minute of silence after a destructive action is indistinguishable
  /// from a hang. The re-check is seconds of git per batch.
  it("says a deletion has started rather than going silent", async () => {
    show();
    await screen.findByText("done");
    fireEvent.click(screen.getByLabelText("done"));
    fireEvent.click(screen.getByRole("button", { name: /^delete 1…/i }));
    await screen.findByText(/where should these be deleted/i);
    fireEvent.click(screen.getByRole("button", { name: /delete 1 branch locally/i }));

    await waitFor(() =>
      expect(toasts.info).toHaveBeenCalledWith(
        expect.stringMatching(/deleting 1 branch/i),
        expect.anything(),
      ),
    );
  });

  /// Local deletion is recoverable from the reflog; a remote one is
  /// not. The default must never be the irreversible option.
  it("defaults a tracked branch to the local-only scope", async () => {
    listFn.mockResolvedValue([
      branch({ name: "shipped", location: "tracked", upstream: "origin/shipped" }),
    ]);
    show();
    await screen.findByText("shipped");
    fireEvent.click(screen.getByLabelText("shipped"));
    fireEvent.click(screen.getByRole("button", { name: /^delete 1…/i }));

    await screen.findByText(/where should these be deleted/i);
    fireEvent.click(screen.getByRole("button", { name: /delete 1 branch locally$/i }));

    await waitFor(() => expect(delLocal).toHaveBeenCalledTimes(1));
    expect(delRemote).not.toHaveBeenCalled();
  });

  /// A refusal names the branch AND the reason: a bare count tells the
  /// user nothing they can act on.
  it("reports each refusal with its reason", async () => {
    delLocal.mockResolvedValue([
      { name: "done", error: "done is not merged: 1 commit(s) are not on the default branch" },
    ]);
    show();
    await screen.findByText("done");
    fireEvent.click(screen.getByLabelText("done"));
    fireEvent.click(screen.getByRole("button", { name: /^delete 1…/i }));
    await screen.findByText(/where should these be deleted/i);
    fireEvent.click(screen.getByRole("button", { name: /delete 1 branch locally/i }));

    await waitFor(() =>
      expect(toasts.error).toHaveBeenCalledWith("Could not delete done", {
        description: "done is not merged: 1 commit(s) are not on the default branch",
      }),
    );
    expect(toasts.success).not.toHaveBeenCalled();
  });

  /// ~9s on a large repository is long enough that a silent page reads
  /// as a hang.
  it("says the scan is slow rather than showing nothing", () => {
    listFn.mockReturnValue(new Promise(() => {}));
    show();
    expect(screen.getByText(/reading branches/i)).toBeTruthy();
  });

  it("asks for a repository when none is selected", () => {
    useFilters.getState().setFilter("repo", undefined);
    show();
    expect(screen.getByText(/select a repository/i)).toBeTruthy();
    expect(listFn).not.toHaveBeenCalled();
  });

  // ---- streaming (#657) -------------------------------------------

  /// A scan in flight: the query never settles, and the page has only
  /// what the stream has given it.
  const streamingScan = (branches: Branch[], total: number, classified: number) => {
    listFn.mockReturnValue(new Promise(() => {}));
    scanState.current = { branches, total, classified };
  };

  /// The point of the whole change. The cold visit used to be ten
  /// seconds of blank page; the listing frame is one `for-each-ref`, so
  /// every row can be on screen before any verdict exists.
  it("renders every row from the stream before any verdict has arrived", () => {
    streamingScan(
      [
        branch({ name: "done", deletable: { kind: "pending" } }),
        branch({ name: "wip", deletable: { kind: "pending" } }),
      ],
      2,
      0,
    );
    show();
    expect(screen.queryByText(/reading branches/i)).toBeNull();
    expect(screen.getByText("done")).toBeTruthy();
    expect(screen.getByText("wip")).toBeTruthy();
    // Exactly the row reasons — `getAllByText` is exact by default, so
    // the status line, which merely quotes the word, is not counted.
    expect(screen.getAllByText("Checking…").length).toBe(2);
  });

  /// A row with no verdict is not permission to delete. The gate is
  /// `merged`, and `pending` is not it — the same rule the backend's
  /// `Deletable::is_deletable` enforces.
  it("does not let a branch with no verdict yet be selected", () => {
    streamingScan(
      [
        branch({ name: "settled" }),
        branch({ name: "waiting", deletable: { kind: "pending" } }),
      ],
      2,
      1,
    );
    show();
    expect(screen.getByLabelText("waiting").hasAttribute("disabled")).toBe(true);
    // A verdict that HAS landed is usable straight away: that is the
    // whole benefit, and each one is complete on its own.
    expect(screen.getByLabelText("settled").hasAttribute("disabled")).toBe(false);
  });

  /// THE test this design exists for.
  ///
  /// The stream dies at 47 of 512 and nothing further ever arrives.
  /// Without the total the page would be indistinguishable from one
  /// that had received everything — 47 rows, some answered, no error.
  /// That is the #701 failure mode: a partial answer reading as a
  /// complete one.
  ///
  /// So the assertion is not "it renders" but that it SAYS SO: the
  /// count is on screen, and it is short of the total.
  it("says the list is incomplete when the stream dies part-way", () => {
    const rows = [
      ...Array.from({ length: 47 }, (_, i) => branch({ name: `settled-${i}` })),
      ...Array.from({ length: 5 }, (_, i) =>
        branch({ name: `stranded-${i}`, deletable: { kind: "pending" } }),
      ),
    ];
    streamingScan(rows, 512, 47);
    show();

    const status = screen.getByRole("status");
    expect(status.textContent).toMatch(/still scanning/i);
    expect(status.textContent).toMatch(/47 of 512/);

    // And it must not offer a sweep of a list it only half has. "All"
    // over a partial list means "all of the ones that turned up".
    const all = screen.getByRole("button", { name: /select all/i }) as HTMLButtonElement;
    expect(all.disabled).toBe(true);
  });

  /// The complement, and the one that would be missed by testing only
  /// the failure: when the scan finishes, the warning goes. A banner
  /// that never clears is one users learn to ignore, which would cost
  /// exactly the legibility it was added for.
  it("drops the incomplete warning once the scan's own answer arrives", async () => {
    scanState.current = {
      branches: [branch({ name: "done", deletable: { kind: "pending" } })],
      total: 1,
      classified: 0,
    };
    listFn.mockResolvedValue([branch({ name: "done" })]);
    show();
    await screen.findByText(/merged \(squashed\)/i);
    expect(screen.queryByRole("status")).toBeNull();
    const all = screen.getByRole("button", { name: /select all/i }) as HTMLButtonElement;
    expect(all.disabled).toBe(false);
  });

  /// The completed scan is the authority. A streamed row must never
  /// outlive it, or the page would show a verdict from a scan the
  /// query has already superseded.
  it("replaces streamed rows with the completed scan when it resolves", async () => {
    scanState.current = {
      branches: [branch({ name: "guessed", deletable: { kind: "pending" } })],
      total: 1,
      classified: 0,
    };
    listFn.mockResolvedValue([branch({ name: "actual" })]);
    show();
    await screen.findByText("actual");
    expect(screen.queryByText("guessed")).toBeNull();
  });

  /// A slow scan that has not even listed yet still says what it is
  /// doing: the listing is fast but not instantaneous, and silence in
  /// that window would be the original complaint in miniature.
  it("still names the wait before the listing frame arrives", () => {
    streamingScan([], 0, 0);
    show();
    expect(screen.getByText(/reading branches/i)).toBeTruthy();
  });

  /// A failed scan shows the failure, never a half-list reading as
  /// data. Belt and braces from both ends: every fallible step in
  /// `scan` — `default_branch` and both `for-each-ref` calls — runs
  /// BEFORE the listing frame, so rows cannot have been streamed when
  /// an error arrives; and the page's error branch returns ahead of the
  /// list regardless. This pins the second half, because the first is
  /// a property of statement order in another language.
  it("shows the failure rather than the rows it managed to stream", async () => {
    scanState.current = {
      branches: [branch({ name: "half", deletable: { kind: "pending" } })],
      total: 9,
      classified: 0,
    };
    listFn.mockRejectedValue("this repository has no origin/HEAD to compare against");
    show();
    expect(await screen.findByText(/no origin\/HEAD/i)).toBeTruthy();
    expect(screen.queryByText("half")).toBeNull();
    expect(screen.queryByRole("status")).toBeNull();
  });
});
