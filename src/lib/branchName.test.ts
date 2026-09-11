import { describe, expect, it } from "vitest";
import { branchNameError, derivedBranchName, soleEcosystem } from "./branchName";

/// A fixed instant, matching `at()` in `packages::apply`'s tests. The
/// generated name ends in a UTC stamp (#797), so a test that let the
/// clock run could assert nothing.
const WHEN = new Date("2026-09-11T06:45:12Z");

/// These mirror `packages::apply::branch_name_at` and
/// `valid_branch_name`. The Rust tests assert the same cases; if the
/// two ever diverge the field shows one name and the run uses another,
/// which is worse than not offering the field.
describe("derivedBranchName", () => {
  /// THE AGREEMENT CASES. `apply.rs`'s
  /// `no_override_keeps_the_derived_name` asserts these three strings
  /// verbatim. Change one side and both test files must change, or the
  /// mirror has stopped being a mirror.
  it("matches the Rust derivation", () => {
    expect(derivedBranchName(["lodash"], "npm", WHEN)).toBe(
      "headstate/npm-deps-20260911-064512",
    );
    expect(derivedBranchName(["a", "b", "c"], "poetry", WHEN)).toBe(
      "headstate/poetry-deps-20260911-064512",
    );
    expect(derivedBranchName([], null, WHEN)).toBe("headstate/deps-20260911-064512");
  });

  /// Pins `the_stamp_is_zero_padded_throughout`. This is the likeliest
  /// place for the two to diverge: chrono pads `%m`/`%d`/`%H` for free,
  /// and the TypeScript side does it by hand.
  it("zero-pads every field of the stamp", () => {
    expect(derivedBranchName([], "uv", new Date("2026-01-02T03:04:05Z"))).toBe(
      "headstate/uv-deps-20260102-030405",
    );
  });

  /// Pins `the_stamp_is_utc`. The desktop derives the name again when the
  /// run starts, so a companion in another timezone must predict the same
  /// string for the same instant -- this would fail on `getMonth()` in
  /// place of `getUTCMonth()`, which is the mistake the getters invite.
  it("stamps in UTC, not local time", () => {
    expect(derivedBranchName([], "npm", new Date("2026-09-11T06:45:12Z"))).toBe(
      derivedBranchName([], "npm", new Date("2026-09-11T08:45:12+02:00")),
    );
  });

  /// The collision #797 is about: the old name keyed on the PACKAGE
  /// COUNT, so two runs touching the same number of packages produced one
  /// branch -- and `create_worktree` refuses an existing one.
  it("gives two runs with the same package count different names", () => {
    const same = ["a", "b", "c"];
    expect(derivedBranchName(same, "poetry", new Date("2026-09-11T06:45:12Z"))).not.toBe(
      derivedBranchName(same, "poetry", new Date("2026-09-11T06:45:13Z")),
    );
  });

  /// A package name can no longer reach the name at all, so a scoped npm
  /// package cannot nest the ref. Kept because `names` is still in the
  /// signature: a future name that interpolates one must not quietly
  /// reintroduce the hazard.
  it("never lets a scoped package name into the ref", () => {
    const b = derivedBranchName(["@scope/pkg"], "npm", WHEN);
    expect(b).toBe("headstate/npm-deps-20260911-064512");
    expect(b).not.toContain("@");
    expect(b.split("/")).toHaveLength(2);
  });

  /// Every generated name must be one git accepts -- in particular a
  /// mixed run must not produce `headstate/-deps-…`, whose leading dash
  /// git reads as an option.
  it("produces a valid ref for every ecosystem, mixed included", () => {
    for (const eco of [null, "npm", "cocoapods", "cargo"] as const) {
      expect(branchNameError(derivedBranchName([], eco, WHEN))).toBeNull();
    }
  });
});

/// Mirrors `packages::apply::sole_ecosystem`.
describe("soleEcosystem", () => {
  it("names an ecosystem only when the selection agrees on one", () => {
    expect(soleEcosystem([])).toBeNull();
    expect(soleEcosystem(["uv", "uv"])).toBe("uv");
    expect(soleEcosystem(["uv", "cargo"])).toBeNull();
  });
});

describe("branchNameError", () => {
  it("accepts ordinary names", () => {
    for (const good of ["update-deps", "headstate/update-lodash", "feature/deps.2026"]) {
      expect(branchNameError(good)).toBeNull();
    }
  });

  /// git's check-ref-format ACCEPTS a leading dash, so only this rule
  /// stands between the name and a git invocation that reads it as an
  /// option.
  it("refuses a name git would read as an option", () => {
    expect(branchNameError("-dashname")).toMatch(/'-'/);
  });

  it("refuses what git itself rejects", () => {
    for (const bad of [
      "",
      "/leading",
      "trailing/",
      "double//slash",
      "ends.",
      "has..dots",
      "thing.lock",
      "at@{brace}",
      "with space",
      "tilde~1",
      "caret^1",
      "colon:here",
      "question?",
      "star*",
      "bracket[",
    ]) {
      expect(branchNameError(bad), `${bad} should be refused`).not.toBeNull();
    }
  });

  it("refuses control characters", () => {
    expect(branchNameError("new\nline")).not.toBeNull();
    expect(branchNameError("tab\there")).not.toBeNull();
  });
});
