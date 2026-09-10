import { describe, expect, it } from "vitest";
import type { Safety } from "@/types/pr";
import {
  canClaudify,
  forceWarning,
  formatSize,
  isSafe,
  pathBasename,
  prForWorktree,
  safetyReason,
  safetyTone,
  totalSize,
} from "./worktrees";

describe("isSafe", () => {
  // Only `safe` is deletable. Everything else is disabled rather than
  // warned past: a cleanup tool that occasionally eats a day of work is
  // worse than no cleanup tool.
  it("is true only for the merged states", () => {
    expect(isSafe({ kind: "safe" })).toBe(true);
    // #732: merged, then the remote branch was deleted. The work is on
    // the default branch, so this is as removable as `safe` -- the
    // whole point of the fix, since treating it as never-pushed left
    // every merged worktree unremovable.
    expect(isSafe({ kind: "merged_upstream_deleted" })).toBe(true);
    for (const s of [
      { kind: "main_checkout" },
      { kind: "dirty", detail: 3 },
      { kind: "unpushed", detail: 2 },
      { kind: "never_pushed" },
      // `empty` included deliberately. Nothing on the branch could be
      // lost, and it is still not one-click removable: #701 reported
      // that the WORDING was wrong, and quietly widening the app's only
      // unrecoverable action on the back of a copy fix is not what was
      // asked for. The forced path stays available.
      { kind: "empty" },
      { kind: "unmerged" },
      { kind: "unknown", detail: "x" },
    ] as Safety[]) {
      expect(isSafe(s)).toBe(false);
    }
  });
});

describe("safetyReason", () => {
  it("pluralises counts", () => {
    expect(safetyReason({ kind: "dirty", detail: 1 })).toBe("1 uncommitted file");
    expect(safetyReason({ kind: "dirty", detail: 3 })).toBe("3 uncommitted files");
    expect(safetyReason({ kind: "unpushed", detail: 1 })).toBe("1 unpushed commit");
  });

  // The most dangerous state deserves the plainest words: 52 of 295
  // worktrees on this machine hold commits that exist nowhere else.
  it("says plainly when commits exist nowhere else", () => {
    expect(safetyReason({ kind: "never_pushed" })).toContain("only here");
    // Both halves matter: "merged" is why the button is enabled, and
    // "upstream deleted" is why no remote branch can be pointed at.
    const gone = safetyReason({ kind: "merged_upstream_deleted" });
    expect(gone).toContain("merged");
    expect(gone).toContain("upstream deleted");
    // It must NOT read like the state it was being confused with.
    expect(gone).not.toContain("only here");
  });

  // The bug in #701: a scratch branch was described as holding commits
  // that exist only here, beside "0 commits ahead". Both cannot be
  // true, and the user believed the scarier one.
  it("does not claim an empty branch holds commits", () => {
    const reason = safetyReason({ kind: "empty" });
    expect(reason).toContain("no commits of its own");
    expect(reason).not.toContain("only here");
    expect(reason).not.toBe(safetyReason({ kind: "never_pushed" }));
  });
});

describe("safetyTone", () => {
  it("uses green only for safe", () => {
    expect(safetyTone({ kind: "safe" })).toContain("3fb950");
    expect(safetyTone({ kind: "unmerged" })).not.toContain("3fb950");
    expect(safetyTone({ kind: "never_pushed" })).not.toContain("3fb950");
    // Green, like `safe`: same verdict, different evidence (#732).
    expect(safetyTone({ kind: "merged_upstream_deleted" })).toContain("3fb950");
  });

  // The main checkout is not a problem, so it must not look like one.
  it("does not alarm about the main checkout", () => {
    expect(safetyTone({ kind: "main_checkout" })).toContain("8b949e");
  });

  // Grey, not red. An empty branch holds no work, so painting it the
  // same colour as "commits exist only here" would repeat #701 in a
  // medium the user reads before the words.
  it("does not alarm about an empty branch", () => {
    expect(safetyTone({ kind: "empty" })).toContain("8b949e");
    expect(safetyTone({ kind: "empty" })).not.toBe(safetyTone({ kind: "never_pushed" }));
  });
});

describe("forceWarning", () => {
  // The confirmation is the moment the user decides, so a false claim
  // there is worse than one on the row.
  it("does not warn about commits an empty branch does not have", () => {
    const warning = forceWarning({ kind: "empty" });
    expect(warning).toContain("nothing on it would be lost");
    expect(warning).not.toContain("not pushed anywhere");
    expect(warning).toContain("cannot be undone");
  });

  it("still names the specific loss for a never-pushed branch", () => {
    expect(forceWarning({ kind: "never_pushed" })).toContain("not pushed anywhere");
  });

  it("falls back to the general form for everything else", () => {
    expect(forceWarning({ kind: "unmerged" })).toContain("does not consider this safe");
  });
});

describe("canClaudify", () => {
  // The rule is "not removable and not the main checkout". `empty` is
  // both, and the gate keeps it un-removable -- so withholding the
  // assess action too would leave the row with no action at all.
  it("offers assessment for an empty branch", () => {
    expect(canClaudify({ kind: "empty" })).toBe(true);
  });

  it("still refuses the states with no question to ask", () => {
    expect(canClaudify({ kind: "safe" })).toBe(false);
    expect(canClaudify({ kind: "main_checkout" })).toBe(false);
    expect(canClaudify({ kind: "pending" })).toBe(false);
  });
});

describe("formatSize", () => {
  it("scales units", () => {
    expect(formatSize(512)).toBe("512 B");
    expect(formatSize(1024)).toBe("1.0 KB");
    expect(formatSize(5 * 1024 * 1024)).toBe("5.0 MB");
    expect(formatSize(3 * 1024 ** 3)).toBe("3.0 GB");
  });

  // An unmeasured size must not read as an empty directory.
  it("shows a dash rather than claiming zero", () => {
    expect(formatSize(null)).toBe("—");
    expect(formatSize(0)).toBe("0 B");
  });
});

describe("pathBasename", () => {
  // The bug: split("/") returns the whole string unchanged on a Windows
  // path, so every row would show the full path instead of the directory.
  it("finds the last component of a Windows path", () => {
    expect(pathBasename("C:\\Users\\me\\code\\proj-feature")).toBe("proj-feature");
  });

  it("finds the last component of a Unix path", () => {
    expect(pathBasename("/Users/me/code/proj-feature")).toBe("proj-feature");
  });

  // Git on Windows often reports forward slashes even for Windows paths,
  // so both must work regardless of which platform produced them.
  it("handles a Windows drive with forward slashes", () => {
    expect(pathBasename("C:/Users/me/code/proj")).toBe("proj");
  });

  it("ignores a trailing separator rather than returning empty", () => {
    expect(pathBasename("/code/proj/")).toBe("proj");
    expect(pathBasename("C:\\code\\proj\\")).toBe("proj");
  });

  it("returns the input when there is no separator at all", () => {
    expect(pathBasename("proj")).toBe("proj");
  });
});

describe("prForWorktree", () => {
  const pr = (repo: string, head: string, number: number) =>
    ({ repo, head_ref: head, number } as unknown as import("@/types/pr").PullRequest);

  it("pairs a worktree with its pull request", () => {
    const prs = [pr("octocat/api", "feat/x", 1)];
    expect(prForWorktree(prs, "octocat/api", "feat/x")?.number).toBe(1);
  });

  // THE trap. Branch names are not unique across repositories -- this
  // account has feat/egr33-* in two of them -- and a wrong pairing would
  // attach GitHub's authoritative-looking "merged" to the wrong
  // directory.
  it("never matches the same branch in a different repository", () => {
    const prs = [pr("octocat/api", "feat/shared", 1)];
    expect(prForWorktree(prs, "octocat/worker", "feat/shared")).toBeNull();
  });

  // A repo with no remote resolves to null identity, which must mean
  // "no pairing" rather than "match anything".
  //
  // The PR fixture below carries repo=null so that a comparison-only
  // implementation WOULD match it -- without that, the guard could be
  // deleted and this test would still pass, since `repo === null` never
  // equals a real repo name.
  it("makes no match when the repository cannot be identified", () => {
    const prs = [
      { repo: null, head_ref: "feat/x", number: 9 } as unknown as
        import("@/types/pr").PullRequest,
    ];
    expect(prForWorktree(prs, null, "feat/x")).toBeNull();
  });

  it("makes no match for a detached worktree", () => {
    const prs = [pr("octocat/api", "feat/x", 1)];
    expect(prForWorktree(prs, "octocat/api", "")).toBeNull();
  });
});

describe("totalSize", () => {
  const w = (size_bytes: number | null) => ({ size_bytes });

  /// THE bug: `reduce(...) || null` turned a real zero into "unknown",
  /// rendering as `—` where `0 B` was the answer.
  it("reports a measured total of zero as zero, not as unknown", () => {
    expect(totalSize([w(0), w(0)])).toBe(0);
  });

  /// The other half: an unmeasured set also summed to 0 and showed the
  /// same dash, so "still measuring" was indistinguishable from
  /// "nothing to reclaim". Sizing is slow, which is why this appeared
  /// only sometimes.
  it("reports nothing-measured as unknown", () => {
    expect(totalSize([w(null), w(null)])).toBeNull();
    expect(totalSize([])).toBeNull();
  });

  it("sums what is known and ignores what is not", () => {
    expect(totalSize([w(100), w(null), w(50)])).toBe(150);
  });

  it("is a real total when everything is measured", () => {
    expect(totalSize([w(1024), w(2048)])).toBe(3072);
  });
});
