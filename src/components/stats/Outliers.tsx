import { ExternalLink } from "../ExternalLink";
import { Card } from "@/components/ui/card";

/// What a row needs to be named and opened.
///
/// A structural type rather than `MergedPr`, so the SAME component serves
/// both the unscoped page (`MergedPr`, snake_case `cycle_time_hours`) and a
/// scope page (`BoardPr`, camelCase `cycleTimeHours`). The cycle time is
/// reached through the `slowestBy` accessor below rather than by a field
/// name, which is the one field the two shapes spell differently -- widening
/// the type here was the alternative to a second copy of this component, and
/// #826 asks for reuse rather than a second idiom.
export interface OutlierPr {
  number: number;
  title: string;
  url: string;
  repo: string;
  size: number;
}

/// The pull requests behind the striking numbers.
///
/// Every figure on this page was a scalar: it could report that something
/// took four days or ran to ten thousand lines and then not say WHICH pull
/// request that was. The repo table already documents the opposite
/// principle for itself -- "a way in, not just a readout" -- and these are
/// the figures that most invite a click.
///
/// Links open github.com rather than filtering the list, because these are
/// MERGED PRs and the list holds open ones.
function OutlierList({
  title,
  hint,
  prs,
  format,
}: {
  title: string;
  hint: string;
  prs: OutlierPr[];
  format: (pr: OutlierPr) => string;
}) {
  if (prs.length === 0) return null;
  return (
    <Card className="px-4">
      <div className="text-sm font-semibold">{title}</div>
      <div className="text-xs text-[#8b949e]">{hint}</div>
      <div className="mt-3 flex flex-col gap-1">
        {prs.map((pr) => (
          <ExternalLink
            key={`${pr.repo}#${pr.number}`}
            href={pr.url}
            className="flex items-baseline gap-3 rounded px-2 py-1.5 text-sm hover:bg-[#161b22]"
          >
            <span className="min-w-0 flex-1 truncate text-[#e6edf3]">{pr.title}</span>
            <span className="shrink-0 text-xs text-[#8b949e]">{pr.repo}</span>
            <span className="shrink-0 tabular-nums text-xs text-[#8b949e]">
              {format(pr)}
            </span>
          </ExternalLink>
        ))}
      </div>
    </Card>
  );
}

export function Outliers<T extends OutlierPr>({
  slowest,
  largest,
  slowestBy,
  hint = "in this sample",
}: {
  slowest: T[];
  largest: T[];
  /// How to read a row's cycle time in hours.
  ///
  /// An accessor because the two callers spell the field differently --
  /// `MergedPr.cycle_time_hours` against `BoardPr.cycleTimeHours` -- and
  /// renaming either would be a change to a serialized Rust type for the
  /// sake of this component. Required rather than defaulted to one spelling,
  /// so a caller cannot silently get 0 for every row by passing the shape
  /// the default does not match.
  slowestBy: (pr: T) => number;
  /// What population these are drawn from, in the caller's words.
  ///
  /// The unscoped page draws from a fixed recent SAMPLE; a scope page draws
  /// from the whole window, complete or labelled. Those are different claims
  /// and the caller is the only one that knows which applies -- a hardcoded
  /// "in this sample" on a complete window would understate the figures.
  hint?: string;
}) {
  return (
    <div className="grid grid-cols-1 gap-3 md:grid-cols-2">
      <OutlierList
        title="Slowest to merge"
        hint={`longest time from open to merge, ${hint}`}
        prs={slowest}
        format={(pr) => {
          const h = slowestBy(pr as T);
          return h >= 24 ? `${(h / 24).toFixed(1)}d` : `${h.toFixed(1)}h`;
        }}
      />
      <OutlierList
        title="Largest changes"
        hint={`most lines added and removed, ${hint}`}
        prs={largest}
        format={(pr) => `${pr.size.toLocaleString()} lines`}
      />
    </div>
  );
}
