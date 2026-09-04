import { describe, expect, it } from "vitest";
import { branchNameError, derivedBranchName } from "./branchName";

/// These mirror `packages::apply::branch_name` and
/// `valid_branch_name`. The Rust tests assert the same cases; if the
/// two ever diverge the field shows one name and the run uses another,
/// which is worse than not offering the field.
describe("derivedBranchName", () => {
  it("matches the Rust derivation", () => {
    expect(derivedBranchName([])).toBe("headstate/updates");
    expect(derivedBranchName(["lodash"])).toBe("headstate/update-lodash");
    expect(derivedBranchName(["a", "b", "c"])).toBe("headstate/updates-3");
  });

  /// Scoped npm packages carry `@` and `/`, and `/` in particular would
  /// nest the ref.
  it("sanitises a scoped package the same way", () => {
    expect(derivedBranchName(["@scope/pkg"])).toBe("headstate/update--scope-pkg");
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
