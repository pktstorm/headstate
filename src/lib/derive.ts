import type { PullRequest, Stats } from "../types/pr";

export const STALE_DAYS = 3;

export interface Filters {
  repo?: string;
  readyOnly?: boolean;
  draftsOnly?: boolean;
  ci?: PullRequest["ci"];
  review?: PullRequest["review"];
  includeLabels?: string[];
  excludeLabels?: string[];
  needsAttentionOnly?: boolean;
  staleOnly?: boolean;
  inMergeQueueOnly?: boolean;
}

/// Blocked on the author and nobody else: a real conflict, or failing CI.
/// `checking` is deliberately excluded -- GitHub reports UNKNOWN
/// mergeability while it computes, and treating that as a conflict would
/// fire a false warning on every push. This mirrors the equivalent rule in
/// the Rust mapping layer -- keep the two consistent.
export function needsAttention(pr: PullRequest): boolean {
  return pr.merge === "conflicted" || pr.ci === "failure";
}

export function awaitingReview(pr: PullRequest): boolean {
  return !pr.is_draft && pr.ci === "success" &&
    (pr.review === "none" || pr.review === "review_required");
}

export function readyToQueue(pr: PullRequest): boolean {
  return !pr.is_draft && pr.ci === "success" && pr.review === "approved" && !pr.in_merge_queue;
}

export function blockedByComments(pr: PullRequest): boolean {
  return pr.review === "changes_requested";
}

/// Green, approved, and untouched for `days`+: the single most
/// nudge-worthy state, and the one no other filter surfaces.
export function isStale(pr: PullRequest, now: Date, days = STALE_DAYS): boolean {
  if (!readyToQueue(pr)) return false;
  const age = now.getTime() - new Date(pr.updated_at).getTime();
  return age > days * 86_400_000;
}

const hasLabel = (pr: PullRequest, names: string[]) =>
  pr.labels.some((l) => names.includes(l.name));

export function applyFilters(
  prs: PullRequest[],
  f: Filters,
  now: Date = new Date(),
): PullRequest[] {
  return prs.filter((pr) => {
    if (f.repo && pr.repo !== f.repo) return false;
    if (f.readyOnly && pr.is_draft) return false;
    if (f.draftsOnly && !pr.is_draft) return false;
    if (f.ci && pr.ci !== f.ci) return false;
    if (f.review && pr.review !== f.review) return false;
    if (f.includeLabels?.length && !hasLabel(pr, f.includeLabels)) return false;
    if (f.excludeLabels?.length && hasLabel(pr, f.excludeLabels)) return false;
    if (f.needsAttentionOnly && !needsAttention(pr)) return false;
    if (f.inMergeQueueOnly && !pr.in_merge_queue) return false;
    if (f.staleOnly && !isStale(pr, now)) return false;
    return true;
  });
}

/// Five of the seven `Stats` fields come from the PR list already in
/// memory, at zero API cost -- the Rust `get_stats` command always returns
/// zero for these because they are not derivable from a single GitHub
/// search. Only `merged_week`/`merged_month` are real and stay Rust-side.
/// Task 17's dashboard must render this, not `get_stats` directly.
export function deriveStats(
  prs: PullRequest[],
): Omit<Stats, "merged_week" | "merged_month"> {
  return {
    in_merge_queue: prs.filter((p) => p.in_merge_queue).length,
    needs_attention: prs.filter(needsAttention).length,
    awaiting_review: prs.filter(awaitingReview).length,
    ready_to_queue: prs.filter(readyToQueue).length,
    blocked_by_comments: prs.filter(blockedByComments).length,
  };
}
