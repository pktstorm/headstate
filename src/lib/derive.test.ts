import { describe, expect, it } from "vitest";
import { PR_FIXTURES } from "../fixtures/prs";
import {
  applyFilters, awaitingReview, blockedByComments, deriveStats,
  isStale, needsAttention, readyToQueue, STALE_DAYS,
} from "./derive";

const [approved, broken, checking] = PR_FIXTURES;

describe("needsAttention", () => {
  it("flags failing CI", () => {
    expect(needsAttention(broken)).toBe(true);
  });

  it("does not flag a green PR", () => {
    expect(needsAttention(approved)).toBe(false);
  });

  /// The priorities strip must never cry wolf: a PR whose mergeability
  /// GitHub has not finished computing is not a conflict.
  it("never flags a PR whose merge state is still checking", () => {
    expect(needsAttention(checking)).toBe(false);
  });
});

describe("isStale", () => {
  it("flags a green approved PR untouched for more than 3 days", () => {
    expect(isStale(approved, new Date("2026-08-25T12:00:00Z"))).toBe(true);
  });

  it("does not flag one touched today", () => {
    expect(isStale(approved, new Date("2026-08-18T13:00:00Z"))).toBe(false);
  });

  it("does not flag a PR that is not yet approved", () => {
    expect(isStale(checking, new Date("2026-08-25T12:00:00Z"))).toBe(false);
  });

  it("honours a custom threshold instead of the STALE_DAYS default", () => {
    const now = new Date("2026-08-19T13:00:00Z"); // 1 day after approved's updated_at
    expect(isStale(approved, now, STALE_DAYS)).toBe(false);
    expect(isStale(approved, now, 1)).toBe(true);
  });
});

describe("categories", () => {
  it("classifies awaiting review, ready to queue, and blocked", () => {
    expect(readyToQueue(approved)).toBe(true);
    expect(blockedByComments(broken)).toBe(true);
    expect(awaitingReview(approved)).toBe(false);
  });
});

describe("applyFilters", () => {
  it("returns everything by default", () => {
    expect(applyFilters(PR_FIXTURES, {}).length).toBe(3);
  });

  it("filters by repo", () => {
    const out = applyFilters(PR_FIXTURES, { repo: "octocat/spoon-knife" });
    expect(out.map((p) => p.number)).toEqual([7]);
  });

  it("hides drafts when readyOnly is set", () => {
    expect(applyFilters(PR_FIXTURES, { readyOnly: true }).some((p) => p.is_draft)).toBe(false);
  });

  it("includes by label", () => {
    const out = applyFilters(PR_FIXTURES, { includeLabels: ["bug"] });
    expect(out.map((p) => p.number)).toEqual([43]);
  });

  /// Excluding `dependencies` to silence dependabot is the dominant
  /// real-world case for label filtering.
  it("excludes by label", () => {
    const out = applyFilters(PR_FIXTURES, { excludeLabels: ["dependencies"] });
    expect(out.map((p) => p.number)).toEqual([42, 43]);
  });

  it("applies include and exclude together", () => {
    const out = applyFilters(PR_FIXTURES, {
      includeLabels: ["bug", "dependencies"],
      excludeLabels: ["dependencies"],
    });
    expect(out.map((p) => p.number)).toEqual([43]);
  });
});

describe("deriveStats", () => {
  it("counts each category from the list", () => {
    const s = deriveStats(PR_FIXTURES);
    expect(s.needs_attention).toBe(1);
    expect(s.in_merge_queue).toBe(1);
    expect(s.blocked_by_comments).toBe(1);
    expect(s.ready_to_queue).toBe(1);
  });
});
