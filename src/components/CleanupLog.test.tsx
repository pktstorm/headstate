import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import type { LedgerEntry } from "@/types/pr";

const runFn = vi.hoisted(() => vi.fn(() => Promise.resolve([] as LedgerEntry[])));
// `loading` is new (#852): the component exposed `isLoading` from the hook
// and never destructured it, so the initial load rendered the empty state's
// DIAGNOSIS -- and this harness had no way to reach that render at all.
const state = vi.hoisted(() => ({ entries: [] as LedgerEntry[], loading: false }));

vi.mock("sonner", () => ({ toast: { success: vi.fn(), error: vi.fn() } }));
vi.mock("../api/hooks", () => ({
  useCleanupLog: () => ({
    entries: state.entries,
    isLoading: state.loading,
    run: runFn,
  }),
}));

import { CleanupLog } from "./CleanupLog";

const entry = (over: Partial<LedgerEntry> = {}): LedgerEntry => ({
  at: new Date().toISOString(),
  kind: "artifact",
  target: "/code/x/target",
  detail: "cargo build",
  bytes: 1_000_000_000,
  action: "proposed",
  error: null,
  ...over,
});

beforeEach(() => {
  runFn.mockClear();
  state.entries = [];
  state.loading = false;
});

describe("CleanupLog", () => {
  it("explains how to get a report when there is none", () => {
    render(<CleanupLog />);
    expect(screen.getByText(/Turn on automatic cleanup in Settings/)).toBeTruthy();
  });

  it("totals only what was proposed", () => {
    state.entries = [
      entry({ bytes: 1_000_000_000 }),
      // Skipped rows must not inflate the headline: the total says what
      // would be reclaimed, and a skipped directory would not be.
      entry({ target: "/code/busy/target", action: "skipped", bytes: 9_000_000_000 }),
    ];
    render(<CleanupLog />);
    expect(screen.getByText(/1 item/)).toBeTruthy();
    expect(screen.queryByText(/10 GB/)).toBeNull();
  });

  /// A row passed over without explanation reads as a malfunction
  /// rather than a guard doing its job.
  it("shows why a row was skipped", () => {
    state.entries = [
      entry({ action: "skipped", error: "written to recently" }),
    ];
    render(<CleanupLog />);
    expect(screen.getByText("written to recently")).toBeTruthy();
  });

  it("runs a pass on demand", async () => {
    render(<CleanupLog />);
    fireEvent.click(screen.getByRole("button", { name: "Check now" }));
    await waitFor(() => expect(runFn).toHaveBeenCalled());
  });

  /// #852: the holding message and the diagnosis are opposite answers.
  ///
  /// `useCleanupLog` has always exposed `isLoading` and this never read it,
  /// so during the initial load the component rendered "Turn on automatic
  /// cleanup in Settings" -- pointing at a setting that is very often
  /// already on. `RepoPickerSidebar` documents the same failure: "'No
  /// repositories found' is a DIAGNOSIS, not a holding message… sends
  /// someone to fix something that is not broken."
  describe("before the ledger has been read", () => {
    it("does not send the user to a setting that may already be on", () => {
      state.loading = true;
      render(<CleanupLog />);
      expect(screen.queryByText(/Turn on automatic cleanup in Settings/)).toBeNull();
      expect(screen.getByText(/reading the cleanup ledger/i)).toBeTruthy();
    });

    /// And the diagnosis comes back once the read has ANSWERED, because
    /// that is when it is true. A holding message that never resolved into
    /// advice would be the same defect in reverse.
    it("gives the advice once the ledger is known to be empty", () => {
      state.loading = false;
      state.entries = [];
      render(<CleanupLog />);
      expect(screen.getByText(/Turn on automatic cleanup in Settings/)).toBeTruthy();
    });
  });

  /// #852: the only silent `slice` in the codebase.
  ///
  /// `StatsSidebar`: "The count is always stated ('Show all 49'), never
  /// silently cut" -- and `PrRow` prints a `+N` with a `title` for the same
  /// reason. It matters most here, because this component's whole purpose is
  /// turning "trust this predicate" into "I have read this list", and a
  /// silently cut audit log cannot do that: the reader believes they have
  /// read the ledger.
  describe("when the ledger is longer than the list", () => {
    const many = (n: number) =>
      Array.from({ length: n }, (_, i) => entry({ target: `/code/p${i}/target` }));

    it("states the cut rather than truncating in silence", () => {
      state.entries = many(62);
      render(<CleanupLog />);
      expect(screen.getByText(/showing 50 of 62/i)).toBeTruthy();
    });

    /// Says BOTH halves. "and 12 more" alone leaves the reader doing
    /// arithmetic to work out whether they have seen the whole ledger,
    /// which is the one question this surface exists to answer.
    it("names how many are not listed", () => {
      state.entries = many(62);
      render(<CleanupLog />);
      expect(screen.getByText(/12 older entries are not listed/i)).toBeTruthy();
    });

    it("renders only the rows it says it is rendering", () => {
      state.entries = many(62);
      render(<CleanupLog />);
      // 50 rows plus the one notice row.
      expect(screen.getAllByRole("listitem")).toHaveLength(51);
    });

    /// Silent when nothing is cut: a permanent "showing 12 of 12" is
    /// furniture, and furniture is what the eye learns to skip.
    it("says nothing when the whole ledger fits", () => {
      state.entries = many(12);
      render(<CleanupLog />);
      expect(screen.queryByText(/showing/i)).toBeNull();
      expect(screen.getAllByRole("listitem")).toHaveLength(12);
    });

    /// The stated cut and the actual one come from ONE constant, so a
    /// footer that misreports its own truncation is impossible. Exercised
    /// at the boundary, where an off-by-one in either place shows up.
    it("does not claim a cut at exactly the limit", () => {
      state.entries = many(50);
      render(<CleanupLog />);
      expect(screen.queryByText(/showing/i)).toBeNull();
      expect(screen.getAllByRole("listitem")).toHaveLength(50);
    });
  });

  /// #852: #818's lock-reason bug, reproduced verbatim on the surface that
  /// exists to audit what cleanup deleted.
  ///
  /// The advisory was `shrink-0` with no width bound, which on a flex row
  /// means "claim my full intrinsic width and give none of it back". The
  /// target was the only `flex-1` cell, so it absorbed all the pressure: a
  /// `refused` entry collapsed the path to nothing and the user could not
  /// tell WHICH item the ledger was discussing. And there was no `title`,
  /// so the clipped text was unrecoverable.
  ///
  /// Asserted on the CLASSES and the `title`, not on geometry: jsdom applies
  /// no stylesheets, so a layout assertion here could only ever check what
  /// the markup declares. `WorktreesPage:238`'s three-part remedy is the
  /// model -- `min-w-0 flex-auto truncate` on the advisory, the full text in
  /// `title`, and `overflow-hidden` on the container.
  describe("a row whose reason is long", () => {
    /// `acme/` is one of the synthetic owners `scripts/check-privacy.sh`
    /// allows. A plausible-looking local checkout path is exactly what that
    /// guard exists to keep out of the repository, and a long target is all
    /// this fixture actually needs -- the length is the point, not the name.
    const TARGET = "/Users/runner/code/acme/a-long-enough-project-to-clip/target";
    const refused = () =>
      entry({
        action: "refused",
        target: TARGET,
        error: `could not remove ${TARGET}: Device or resource busy`,
      });

    it("lets the reason yield width instead of squeezing the target out", () => {
      state.entries = [refused()];
      render(<CleanupLog />);
      const reason = screen.getByText(/Device or resource busy/);
      // It SHRINKS and CLIPS -- the half that was missing.
      expect(reason.className).toContain("min-w-0");
      expect(reason.className).toContain("truncate");
      // And it is no longer the cell that refuses to give width back.
      expect(reason.className).not.toContain("shrink-0");
    });

    /// `flex-auto`, not `flex-1`, for `WorktreesPage`'s reason: `flex-1` is
    /// `flex: 1 1 0%` and would give the reason an equal share of the row
    /// however short it is, leaving a two-word refusal in a half-row of
    /// whitespace.
    it("sizes the reason from its content rather than claiming half the row", () => {
      state.entries = [refused()];
      render(<CleanupLog />);
      const reason = screen.getByText(/Device or resource busy/);
      expect(reason.className).toContain("flex-auto");
      expect(reason.className).not.toMatch(/\bflex-1\b/);
    });

    /// The tooltip is the ONLY place the truncated tail survives, so it has
    /// to be complete -- #818's conclusion for the worktree row.
    it("keeps the full reason and the full target recoverable", () => {
      state.entries = [refused()];
      render(<CleanupLog />);
      const reason = screen.getByText(/Device or resource busy/);
      expect(reason.getAttribute("title")).toBe(refused().error);
      const target = screen.getByText(refused().target);
      expect(target.getAttribute("title")).toBe(refused().target);
    });

    /// The third part: without `overflow-hidden` a cell that overflows its
    /// share draws straight through the bordered box, so even a correctly
    /// shrinking neighbour cannot stop the row spilling.
    it("clips the row at its own border", () => {
      state.entries = [refused()];
      render(<CleanupLog />);
      const row = screen.getByText(/Device or resource busy/).closest("li") as HTMLElement;
      expect(row.className).toContain("overflow-hidden");
    });

    /// The ORDER is the priority #818 settled: the identifier keeps the
    /// room, the sentence gives it up. Asserted with
    /// `compareDocumentPosition` rather than on the parent alone, because a
    /// parent-only assertion is what let #817 ship green.
    it("puts the target before the reason, so the identifier leads", () => {
      state.entries = [refused()];
      render(<CleanupLog />);
      const target = screen.getByText(refused().target);
      const reason = screen.getByText(/Device or resource busy/);
      expect(
        target.compareDocumentPosition(reason) & Node.DOCUMENT_POSITION_FOLLOWING,
      ).toBeTruthy();
    });
  });
});
