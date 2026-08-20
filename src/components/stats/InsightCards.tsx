import { Card } from "@/components/ui/card";
import { percentile } from "@/lib/stats";
import type { MergedDetail } from "@/types/pr";

function Stat({
  label,
  value,
  hint,
}: {
  label: string;
  value: string;
  hint: string;
}) {
  return (
    <Card className="px-4">
      <div className="text-xs text-[#8b949e]">{label}</div>
      <div className="mt-1 text-2xl font-semibold tabular-nums">{value}</div>
      <div className="mt-1 text-xs text-[#8b949e]">{hint}</div>
    </Card>
  );
}

/// Quality signals over the merged-PR sample.
///
/// Every per-PR figure divides by `sample_size`, so an empty sample renders
/// "--" rather than NaN. The hints name the sample explicitly: these are
/// aggregates over the last N merged PRs, not lifetime totals, and a reader
/// who mistakes one for the other draws the wrong conclusion.
export function InsightCards({ detail }: { detail: MergedDetail }) {
  const n = detail.sample_size;
  const lines = detail.additions + detail.deletions;
  const per = (total: number, digits = 1) =>
    n === 0 ? "--" : (total / n).toFixed(digits);
  const median = percentile(detail.cycle_time_hours, 0.5);
  const p90 = percentile(detail.cycle_time_hours, 0.9);
  const hasTimes = detail.cycle_time_hours.length > 0;

  return (
    <div className="grid grid-cols-1 gap-3 md:grid-cols-3">
      <Stat
        label="Cycle time"
        value={hasTimes ? `${median.toFixed(1)}h` : "--"}
        hint={hasTimes ? `median · p90 ${p90.toFixed(1)}h` : "no timing data"}
      />
      <Stat
        label="Lines changed"
        value={lines.toLocaleString()}
        hint={
          n === 0
            ? "no sample"
            : `${per(lines, 0)} per PR · ${per(detail.changed_files)} files`
        }
      />
      <Stat
        label="Review burden"
        value={per(detail.comment_count)}
        hint={
          n === 0
            ? "no sample"
            : `comments per PR · ${per(detail.review_count)} reviews`
        }
      />
    </div>
  );
}
