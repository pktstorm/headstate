/// The pure logic behind the System Health view: deciding where a
/// series has holes in it, and turning raw readings into the words and
/// colours the panels show.
///
/// Separated from `SystemHealthPage` the way `lib/worktrees.ts` and
/// `lib/docker.ts` are separated from their views. Two reasons, and the
/// second is the one that matters: a module of plain functions is
/// testable without rendering anything, and the gap detection below is
/// the single most important piece of correctness in the whole view --
/// it is what decides whether the chart tells the truth.

/// The interval the Rust sampler writes at, in milliseconds.
///
/// Mirrors the 60-second sleep in `src-tauri/src/lib.rs`. Used as the
/// floor for what counts as ordinary spacing, so it must track that
/// loop; if the sampler's cadence changes, this changes with it.
export const SAMPLE_INTERVAL_MS = 60_000;

/// How many times the typical spacing makes a gap.
///
/// `store::health::history` downsamples by taking every Nth ROW, so two
/// adjacent points in a dense series are legitimately N minutes apart
/// -- that spacing is the resolution, not a hole. So the threshold is
/// derived from the series' own median spacing rather than from the raw
/// 60s cadence, and this multiplier is what separates "one sample
/// further apart than usual" from "the app was closed".
const GAP_FACTOR = 2.5;

/// A spacing that is a gap however typical it is for this series.
///
/// The relative threshold alone has a blind spot: a series of two
/// points six hours apart has a MEDIAN spacing of six hours, so nothing
/// in it exceeds `median * GAP_FACTOR` and the pair would join into a
/// line across a period nobody measured -- exactly the claim this
/// module exists to refuse.
///
/// Thirty minutes is comfortably above the coarsest legitimate spacing
/// the backend can produce: `MAX_POINTS` is 120 across a 24-hour
/// retention window, so even a completely full series is bucketed at
/// twelve minutes and nothing regular is ever wider than that.
const ABSOLUTE_GAP_MS = 30 * 60_000;

/// One point of a series: a time, and a value that may be absent.
export interface Point {
  /// Epoch milliseconds.
  t: number;
  /// `null` where the sample carried no reading. Such a point breaks
  /// the line exactly like a gap does -- for the same reason.
  v: number | null;
}

/// The median spacing between consecutive points, in ms.
///
/// The MEDIAN, not the mean: the mean of a series with a six-hour hole
/// in it is dragged upward by that hole, so the very gap being looked
/// for would raise the threshold above itself and hide. The median of a
/// mostly-regular series is simply the regular spacing.
export function medianSpacing(points: Point[]): number {
  if (points.length < 2) return SAMPLE_INTERVAL_MS;
  const deltas: number[] = [];
  for (let i = 1; i < points.length; i += 1) deltas.push(points[i].t - points[i - 1].t);
  deltas.sort((a, b) => a - b);
  const mid = Math.floor(deltas.length / 2);
  const median =
    deltas.length % 2 === 0 ? (deltas[mid - 1] + deltas[mid]) / 2 : deltas[mid];
  // Never below the sampler's own cadence: a very short series can have
  // a median of a few seconds if two rows land close together, and a
  // threshold derived from that would call every ordinary minute a gap.
  return Math.max(median, SAMPLE_INTERVAL_MS);
}

/// Split a series into the runs that were actually measured.
///
/// **This is the honesty requirement of the whole view.** The sampler
/// runs only while the app is open, so a laptop closed overnight leaves
/// two points twelve hours apart. Drawing a segment between them claims
/// the metric held that value all night -- a fact nobody collected. So
/// the series is cut wherever the spacing exceeds the threshold, and
/// each run is drawn as its own polyline with nothing between them.
///
/// A `null` VALUE cuts the run too. "The platform did not report CPU
/// for these ten minutes" is the same kind of unknown as "the app was
/// not running", and interpolating across it would be the same lie in a
/// different costume.
///
/// Runs of a single point survive: one measurement is still a
/// measurement, and the caller draws it as a dot rather than dropping
/// it, which would under-report what the app actually saw.
export function splitOnGaps(points: Point[]): Point[][] {
  // The tighter of the two rules. Relative spacing handles a
  // downsampled series, whose stride is legitimately wide; the absolute
  // ceiling handles a series too short for its own median to mean
  // anything.
  const threshold = Math.min(medianSpacing(points) * GAP_FACTOR, ABSOLUTE_GAP_MS);
  const runs: Point[][] = [];
  let run: Point[] = [];
  for (const p of points) {
    if (p.v === null) {
      // Not measured: close whatever run was open and skip the point.
      if (run.length > 0) runs.push(run);
      run = [];
      continue;
    }
    const prev = run[run.length - 1];
    if (prev && p.t - prev.t > threshold) {
      runs.push(run);
      run = [];
    }
    run.push(p);
  }
  if (run.length > 0) runs.push(run);
  return runs;
}

/// The colour a utilisation bar takes at a given percentage.
///
/// Three bands rather than a gradient: the question a user asks of this
/// view is "is anything about to run out", which has a yes/nearly/no
/// answer. The thresholds are deliberately high -- a machine at 70%
/// memory is a machine being used, not a machine in trouble, and
/// colouring that amber trains people to ignore the colour, which costs
/// the red band the only thing it has.
export function barColor(percent: number): string {
  if (percent >= 90) return "#f85149";
  if (percent >= 75) return "#d29922";
  return "#3fb950";
}

/// The colour of a thermal pressure label.
///
/// `nominal` is grey rather than green on purpose: green reads as an
/// achievement, and a machine at rest is the ordinary case, not a good
/// one. The escalation is where the colour earns attention.
export function thermalColor(label: string): string {
  switch (label) {
    case "critical":
      return "#f85149";
    case "serious":
      return "#d29922";
    case "fair":
      return "#58a6ff";
    default:
      return "#8b949e";
  }
}

/// What each thermal pressure label means, in one short phrase.
///
/// The labels come from the platform and are not self-explanatory:
/// "serious" alone does not tell anyone whether to close something.
export const THERMAL_MEANING: Record<string, string> = {
  nominal: "Running cool; nothing is being held back.",
  fair: "Warm. The system may be trimming performance slightly.",
  serious: "Hot. The system is actively slowing itself to cope.",
  critical: "Very hot. Performance is heavily limited.",
};

/// Seconds since boot, in words a person reads.
export function formatUptime(secs: number): string {
  const d = Math.floor(secs / 86_400);
  const h = Math.floor((secs % 86_400) / 3600);
  const m = Math.floor((secs % 3600) / 60);
  if (d > 0) return `${d}d ${h}h`;
  if (h > 0) return `${h}h ${m}m`;
  return `${m}m`;
}

/// Percentage of a total, or `null` when the total is zero.
///
/// Zero total means the platform reported nothing, and the two failures
/// this prevents are both real: a disk of size 0 is not a full disk,
/// and `used / 0` is NaN, which renders as the string "NaN%" and looks
/// like a bug because it is one.
export function percentOf(part: number, total: number): number | null {
  if (!total || !Number.isFinite(total)) return null;
  return (part / total) * 100;
}
