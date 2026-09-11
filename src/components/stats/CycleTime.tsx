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
/// # Why `prs` is a separate prop from `hours.length`
///
/// They are genuinely different numbers and the gap is information. `hours`
/// holds one entry per MERGED pull request; `prs` counts merged and open
/// alike, because an open pull request is still work. So a person with ten
/// pull requests and three cycle times has seven still open -- and the card
/// says so, rather than leaving a reader to assume the median covers all ten.
///
/// Passing `hours.length` for `prs` would make that line read "3 of 3
/// merged", which is true of the distribution and false about the person.
export function CycleTime({ hours, prs }: { hours: number[]; prs: number }) {
  if (hours.length === 0) {
    return (
      <Card className="px-4">
        <div className="text-xs text-[#8b949e]">Cycle time</div>
        <div className="mt-1 text-2xl font-semibold tabular-nums text-[#8b949e]">
          --
        </div>
        {/* Not "no data": there IS data, and what it says is that none of
            these pull requests merged in this window. A median over an empty
            set is undefined, and saying why beats a bare dash. */}
        <div className="mt-1 text-xs text-[#8b949e]">
          {prs === 0
            ? "no pull requests in this window"
            : `none of ${prs === 1 ? "this pull request" : `these ${prs} pull requests`} merged in this window`}
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
  const open = prs - hours.length;
  const fmt = (h: number) => (h >= 24 ? `${(h / 24).toFixed(1)}d` : `${h.toFixed(1)}h`);

  return (
    <Card className="px-4">
      <div className="text-xs text-[#8b949e]">Cycle time</div>
      <div className="mt-1 text-2xl font-semibold tabular-nums">{fmt(median)}</div>
      <div className="mt-1 text-xs text-[#8b949e]">
        median over {hours.length} merged · {tailLabel} {fmt(tail)}
        {/* The open count, when there is one. A median over the merged half
            of someone's work is a different figure from a median over all of
            it, and this is the only place that difference is visible. */}
        {open > 0
          ? ` · ${open} still open, not counted`
          : ""}
      </div>
    </Card>
  );
}
