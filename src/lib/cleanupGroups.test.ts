import { describe, expect, it } from "vitest";
import type { CleanupPrefs } from "@/types/pr";
import { CLEANUP_GROUPS, parentState, toggleChild, toggleParent } from "./cleanupGroups";

const base: CleanupPrefs = {
  enabled: false,
  mode: "preview",
  artifacts: false,
  venvs: false,
  venvs_stale: false,
  branches: false,
  branches_ancestor: false,
  branches_squash: false,
  worktrees: false,
  worktrees_safe: false,
  docker: false,
  docker_dangling: false,
  max_per_run: 0,
};

const branches = CLEANUP_GROUPS.find((g) => g.key === "branches")!;
const artifacts = CLEANUP_GROUPS.find((g) => g.key === "artifacts")!;

describe("parentState", () => {
  it("is off when the parent is off", () => {
    expect(parentState(base, branches)).toBe("off");
  });

  it("is on when the parent and every child are on", () => {
    const p = { ...base, branches: true, branches_ancestor: true, branches_squash: true };
    expect(parentState(p, branches)).toBe("on");
  });

  /// A parent rendered as plain "on" while only some children were
  /// would misstate what the unattended pass will do — the one thing
  /// these controls exist to be precise about.
  it("is mixed when only some children are on", () => {
    const p = { ...base, branches: true, branches_ancestor: true };
    expect(parentState(p, branches)).toBe("mixed");
  });

  it("is off when the parent is on but no child is", () => {
    expect(parentState({ ...base, branches: true }, branches)).toBe("off");
  });

  it("treats a childless category as a plain checkbox", () => {
    expect(parentState({ ...base, artifacts: true }, artifacts)).toBe("on");
    expect(parentState(base, artifacts)).toBe("off");
  });
});

describe("toggleParent", () => {
  it("ticking a parent ticks every child", () => {
    expect(toggleParent(base, branches)).toEqual({
      branches: true,
      branches_ancestor: true,
      branches_squash: true,
    });
  });

  it("unticking a fully-on parent unticks every child", () => {
    const p = { ...base, branches: true, branches_ancestor: true, branches_squash: true };
    expect(toggleParent(p, branches)).toEqual({
      branches: false,
      branches_ancestor: false,
      branches_squash: false,
    });
  });

  /// Clicking a mixed parent means "all of these", not "undo the ones
  /// I already picked".
  it("a mixed parent turns everything on", () => {
    const p = { ...base, branches: true, branches_ancestor: true };
    expect(toggleParent(p, branches)).toEqual({
      branches: true,
      branches_ancestor: true,
      branches_squash: true,
    });
  });
});

describe("toggleChild", () => {
  /// Without this a user could tick a child under an off parent and
  /// see nothing happen, because the pass reads the parent.
  it("ticking a child turns its parent on", () => {
    expect(toggleChild(base, branches, "branches_squash")).toEqual({
      branches_squash: true,
      branches: true,
    });
  });

  it("unticking the last child clears the parent", () => {
    const p = { ...base, branches: true, branches_squash: true };
    expect(toggleChild(p, branches, "branches_squash")).toEqual({
      branches_squash: false,
      branches: false,
    });
  });

  it("unticking one of several leaves the parent on", () => {
    const p = { ...base, branches: true, branches_ancestor: true, branches_squash: true };
    expect(toggleChild(p, branches, "branches_squash")).toEqual({
      branches_squash: false,
      branches: true,
    });
  });
});

/// Every group's fields must exist on CleanupPrefs, or a checkbox
/// writes a key the backend drops on the next save.
describe("the group definitions", () => {
  it("names only real preference fields", () => {
    for (const g of CLEANUP_GROUPS) {
      expect(g.key in base).toBe(true);
      for (const c of g.children) expect(c.key in base).toBe(true);
    }
  });
});
