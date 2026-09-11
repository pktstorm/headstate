import { Card } from "@/components/ui/card";
import { percentile } from "@/lib/stats";

/// Cycle time for one person in one scope: median and tail.
///
/// # Why the tail label changes with the sample size
///
/// `InsightCards` established this and the reason holds here: for a small
/// distribution, "p90" resolves to the largest value in it, so a brand-new
/// account's single weekend pull request would be presented with the
/// authority of a tail metric. Under twenty values the label says "slowest"
/// instead of implying a distribution that is not there.
///
/// # Why `prs` is a separate prop from `hours.length`, and what the gap means
///
/// They are different numbers and the gap is information, but NOT the
/// information a first reading suggests -- this was corrected in review and
/// the correction matters, because the wrong label accused a pull request of
/// being slow when it was merged.
///
/// On a board the two differ only on a PARSE FAILURE. The board's query is
/// `is:pr is:merged merged:<range>` (`scope.rs`), so every node it sees has a
/// non-null `mergedAt` and `prs` counts merged pull requests only. A row's
/// `cycleTimeHours` is therefore shorter than its `prs` exactly when Rust's
/// `cycle_hours` returned `None` -- an unparseable timestamp, or a merge
/// recorded before its own creation, both of which it drops rather than
/// clamping to a number nobody can explain.
///
/// So the label says "excluded, no usable merge time" and not "still open".
/// The first version said the latter, which described a mixed open-and-merged
/// population the merged-only board never produces -- and would have read, to
/// anyone checking, as the card calling a merged pull request unfinished.
///
/// `prs` stays a separate prop rather than being dropped, because the gap is
/// worth surfacing even when it is rare: a silently shorter distribution is a
/// median over a population the reader cannot see the size of.
export function CycleTime({ hours, prs }: { hours: number[]; prs: number }) {
  if (hours.length === 0) {
    return (
      <Card className="px-4">
        <div className="text-xs text-[#8b949e]">Cycle time</div>
        <div className="mt-1 text-2xl font-semibold tabular-nums text-[#8b949e]">
          --
        </div>
        {/* Not "no data": a median over an empty set is undefined, and saying
            why beats a bare dash. Which reason applies depends on `prs`, and
            neither is "none of these merged" -- a merged-only board has no
            unmerged pull requests in it, so a non-zero `prs` with no cycle
            times means the timestamps were unusable. */}
        <div className="mt-1 text-xs text-[#8b949e]">
          {prs === 0
            ? "no pull requests in this window"
            : `no usable merge time on ${prs === 1 ? "the one pull request" : `any of these ${prs} pull requests`}`}
        </div>
      </Card>
    );
  }
  // `hours` arrives sorted ascending from Rust, which is the contract
  // `percentile()` indexes against -- see `AuthorRow::cycle_time_hours`. Not
  // re-sorted here: a second sort would hide a future regression in that
  // contract rather than letting it fail visibly.
  const median = percentile(hours, 0.5);
  const tail = percentile(hours, 0.9);
  const tailLabel = hours.length >= 20 ? "p90" : "slowest";
  // Pull requests with no USABLE merge time, which on a merged-only board is
  // a parse failure rather than an open pull request. See the doc above.
  const unusable = prs - hours.length;
  const fmt = (h: number) => (h >= 24 ? `${(h / 24).toFixed(1)}d` : `${h.toFixed(1)}h`);

  return (
    <Card className="px-4">
      <div className="text-xs text-[#8b949e]">Cycle time</div>
      <div className="mt-1 text-2xl font-semibold tabular-nums">{fmt(median)}</div>
      <div className="mt-1 text-xs text-[#8b949e]">
        median over {hours.length} merged · {tailLabel} {fmt(tail)}
        {/* The excluded count, when there is one. NOT "still open": on a
            merged-only board every pull request has merged, so a gap here is
            a timestamp the app could not use -- and calling a merged pull
            request unfinished is the opposite of what happened. */}
        {unusable > 0
          ? ` · ${unusable} excluded, no usable merge time`
          : ""}
      </div>
    </Card>
  );
}
