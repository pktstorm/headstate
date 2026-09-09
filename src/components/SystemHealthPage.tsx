import { useMemo, useState } from "react";
import {
  useAllWorktreeSizes,
  useArtifactSizes,
  useArtifacts,
  useDockerDiskUsage,
  useSystemFootprint,
  useSystemHealth,
  useSystemHealthHistory,
  useVenvSizes,
  useVenvs,
  useWorktrees,
} from "@/api/hooks";
import { QueryError, errorMessage } from "./QueryError";
import { formatSize } from "@/lib/worktrees";
import {
  THERMAL_MEANING,
  barColor,
  formatUptime,
  percentOf,
  splitOnGaps,
  thermalColor,
  type Point,
} from "@/lib/health";
import type { FootprintProcess, HealthGpu, HealthSample } from "@/types/pr";
import { IS_MOBILE_BUILD } from "@/lib/target";
import { useConnectionState } from "@/api/connection";

/// The machine's health: what it is doing now, and the last 24 hours.
///
/// # This view has no repository
///
/// Every other view is scoped by the repo the sidebar selected. This one
/// describes the MACHINE, so a repo picker beside it would be a control
/// that changes nothing -- see `App.tsx`, which renders no sidebar repo
/// list and no `FilterBar` for it.
///
/// # Whose machine
///
/// On the phone this view describes the PAIRED DESKTOP, never the phone
/// itself. A companion reporting its own battery would be answering a
/// question nobody asked: the user opened it to check on the machine
/// they left running.
///
/// The wording is switched on `IS_MOBILE_BUILD`, not on `useIsMobile()`.
/// Which machine is being described is a fact about the BUILD, and
/// `useIsMobile()` is also true for a narrow desktop window -- where a
/// page headed "the desktop's health" would be describing the very
/// machine it is running on. See the ground rules on #590.
///
/// # Two rules the panels below exist to keep
///
/// 1. **Absent is not zero.** Every optional field on the Rust `Sample`
///    is `null` when the platform does not expose it, and this file
///    renders every one of those as "Not measured". A 0 in that slot
///    would be a measurement the app never took, which is the same
///    class of lie as reporting "no updates" for a check that failed
///    to run.
/// 2. **Gaps are gaps.** The sampler only runs while the app is open,
///    so the series has holes. `splitOnGaps` (in `lib/health`) breaks
///    the line across them rather than joining two points hours apart,
///    because the whole value of a 24-hour chart is being able to trust
///    the shape of the line.
///
/// The pure logic behind both rules lives in `@/lib/health`, so it is
/// testable without rendering; this file is the layout.

/// Nothing measured, said the same way everywhere.
///
/// One component rather than a repeated string so that the phrase, the
/// colour, and the fact that it is NOT a number are decided in one
/// place. Grey, deliberately: an absent reading is not a warning, and
/// amber would tell the user to act on something the app simply did
/// not look at.
function NotMeasured({ children }: { children?: React.ReactNode }) {
  return (
    <span className="text-[#8b949e]">
      Not measured{children ? <span className="ml-1">{children}</span> : null}
    </span>
  );
}

/// A labelled figure, with the absent case handled once.
function Stat({
  label,
  value,
  hint,
}: {
  label: string;
  /// `null` means "the platform did not report this", and renders as
  /// `NotMeasured`. It is never coerced to 0.
  value: string | null;
  hint?: string;
}) {
  return (
    <div className="min-w-24">
      <div className="text-xs text-[#8b949e]">{label}</div>
      <div className="text-sm font-medium tabular-nums text-[#e6edf3]">
        {value === null ? <NotMeasured /> : value}
      </div>
      {hint ? <div className="text-xs text-[#8b949e]">{hint}</div> : null}
    </div>
  );
}

/// A horizontal utilisation bar.
///
/// `percent` is clamped rather than trusted: a disk can report more
/// used than total during a snapshot, and a bar wider than its track
/// would break the layout for a rounding artefact.
function Bar({ percent, label }: { percent: number; label?: string }) {
  const clamped = Math.max(0, Math.min(100, percent));
  return (
    <div
      className="h-2 w-full overflow-hidden rounded-full bg-[#30363d]"
      role="meter"
      aria-valuenow={Math.round(clamped)}
      aria-valuemin={0}
      aria-valuemax={100}
      aria-label={label}
    >
      <div
        className="h-full rounded-full"
        style={{ width: `${clamped}%`, backgroundColor: barColor(clamped) }}
      />
    </div>
  );
}

/// A panel: the repeated card shell, so the five below cannot drift.
function Panel({
  title,
  subtitle,
  children,
}: {
  title: string;
  subtitle?: React.ReactNode;
  children: React.ReactNode;
}) {
  return (
    <section className="rounded-md border border-[#30363d] bg-[#161b22] p-4">
      <h2 className="text-sm font-semibold text-[#e6edf3]">{title}</h2>
      {subtitle ? <p className="mt-0.5 text-xs text-[#8b949e]">{subtitle}</p> : null}
      <div className="mt-3">{children}</div>
    </section>
  );
}

/// A 24-hour sparkline that draws its gaps as gaps.
///
/// Hand-rolled SVG rather than the recharts setup `stats/ActivityChart`
/// uses. Recharts joins a series across a `null` unless every gap is
/// materialised as an explicit null-valued row, and the series here is
/// not evenly spaced at all -- the x axis is real time, and the gaps
/// are the point. One polyline per measured run says exactly that, with
/// no library configuration standing between the data and the picture.
function Sparkline({
  points,
  max,
  label,
  now,
  color = "#58a6ff",
}: {
  points: Point[];
  /// The top of the y axis. Passed in rather than derived so a CPU
  /// chart is always 0-100 and does not silently rescale to make a 4%
  /// blip look like a spike.
  max: number;
  label: string;
  /// The right-hand edge of the 24-hour window, in epoch ms.
  ///
  /// Passed in rather than read from `Date.now()` here, for two
  /// reasons. It keeps this a pure function of its props -- the lint
  /// rule that caught it is right, since a re-render would otherwise
  /// shift the axis under an unchanged series. And it lets the caller
  /// anchor the window on the LIVE SAMPLE's own timestamp, so the
  /// chart's "now" is the moment the app actually measured rather than
  /// the moment React happened to paint.
  now: number;
  color?: string;
}) {
  const runs = useMemo(() => splitOnGaps(points), [points]);
  const measured = runs.reduce((n, r) => n + r.length, 0);

  if (measured === 0) {
    return (
      <div
        className="flex h-16 items-center justify-center rounded border border-dashed border-[#30363d] text-xs"
        role="img"
        aria-label={`${label}: no history yet`}
      >
        <NotMeasured>— no history yet</NotMeasured>
      </div>
    );
  }

  // A fixed viewBox with `preserveAspectRatio="none"`: the chart
  // stretches to whatever width the panel has, and the maths below
  // stays in one coordinate system.
  const W = 300;
  const H = 48;
  // The window is always the full 24 hours the backend retains, not
  // the extent of the data. Otherwise ten minutes of history would
  // stretch across the whole width and read as a day of it -- and the
  // gap at the start, which is exactly "we were not running", would
  // vanish by rescaling.
  //
  // Anchored on `now` rather than on the newest point, so a series that
  // stops six hours ago sits at the LEFT of the window with six hours
  // of empty band after it. Ending the axis at the last point would
  // push that stretch off the chart entirely -- and "we have not
  // measured anything since lunchtime" is precisely the fact this view
  // must not hide.
  const end = now;
  const start = end - 24 * 60 * 60 * 1000;
  const span = end - start;
  const x = (t: number) => ((t - start) / span) * W;
  const y = (v: number) => H - (Math.max(0, Math.min(max, v)) / max) * H;

  return (
    <svg
      viewBox={`0 0 ${W} ${H}`}
      preserveAspectRatio="none"
      className="h-16 w-full"
      role="img"
      aria-label={`${label} over the last 24 hours, ${runs.length} measured ${
        runs.length === 1 ? "period" : "periods"
      }`}
      data-testid={`sparkline-${label}`}
      // Read by the test, and by anyone wondering whether a break in
      // the line is real: it is the count of separate runs the data
      // actually contains.
      data-runs={runs.length}
    >
      {/* A faint band behind the whole window, so a short line reads as
          "we measured this much of the day" rather than as the whole
          day. The unmeasured stretches are the band showing through --
          the gap is visible as absence, which is what it is. */}
      <rect x={0} y={0} width={W} height={H} fill="#0d1117" />
      {runs.map((run) => {
        const key = `${run[0].t}-${run[run.length - 1].t}`;
        if (run.length === 1) {
          // One point is not a line. Drawn as a dot rather than dropped:
          // a single measurement between two long closures is real, and
          // silently discarding it would under-report the history.
          return (
            <circle
              key={key}
              cx={x(run[0].t)}
              cy={y(run[0].v as number)}
              r={1.5}
              fill={color}
            />
          );
        }
        return (
          <polyline
            key={key}
            fill="none"
            stroke={color}
            strokeWidth={1.5}
            vectorEffect="non-scaling-stroke"
            points={run.map((p) => `${x(p.t)},${y(p.v as number)}`).join(" ")}
          />
        );
      })}
    </svg>
  );
}

/// One process, as the live half of the footprint panel lists it.
///
/// CPU is NOT clamped to 100 and NOT drawn as a `Bar`, unlike every
/// other percentage on this page. `Footprint.cpu_percent` is a share of
/// one core, so a `git` using three of them legitimately reads 280% --
/// a bar would peg at full and hide exactly the fan-out worth seeing,
/// and clamping the number would report a busy process as a saturated
/// one.
function ProcessRow({ p, hint }: { p: FootprintProcess; hint?: string }) {
  return (
    <tr className="border-t border-[#30363d]">
      <td className="py-1 pr-2">
        <span className="text-[#e6edf3]">{p.name}</span>
        {hint ? <span className="ml-2 text-xs text-[#8b949e]">{hint}</span> : null}
      </td>
      {/* The PID is here because this panel's job ends at naming the
          cost: a user who wants to know what a 900%-CPU `git` is doing
          needs something to type into `ps`, and the name alone does not
          identify one of forty. */}
      <td className="py-1 pr-2 text-right text-xs tabular-nums text-[#8b949e]">
        {p.pid}
      </td>
      <td className="py-1 pr-2 text-right tabular-nums text-[#e6edf3]">
        {p.cpu_percent.toFixed(0)}%
      </td>
      <td className="py-1 text-right tabular-nums text-[#e6edf3]">
        {formatSize(p.memory)}
      </td>
    </tr>
  );
}

/// The header row shared by the process tables, so the two cannot drift.
function ProcessHead() {
  return (
    <thead>
      <tr className="text-left text-xs text-[#8b949e]">
        <th className="font-normal">Process</th>
        <th className="font-normal text-right">PID</th>
        {/* Named "CPU (of one core)" rather than "CPU" precisely
            because the number goes above 100. A reader who sees 280%
            under a bare "CPU" heading concludes the app is broken; the
            heading is the cheapest place to say what the unit is. */}
        <th className="font-normal text-right">CPU (of one core)</th>
        <th className="font-normal text-right">Memory</th>
      </tr>
    </thead>
  );
}

/// A disk figure, and the state it is in.
///
/// FOUR states, not two, and the ones that are not a number are the
/// reason this exists: a figure can be un-measured, measuring, measured,
/// or measured-and-there-was-nothing. Rendering the first as `0 B` is
/// the "absent is never zero" rule applied to the disk half -- "we have
/// not walked your worktrees yet" and "your worktrees are empty" are
/// opposite claims, and the second one would send someone looking for
/// files that are exactly where they left them.
///
/// The fourth state is the mirror of that mistake. Once the scan HAS
/// run and found nothing, saying "not measured" reports a completed
/// look as a failure to look -- so an honest "none found" belongs
/// there, and only there.
function DiskRow({
  label,
  bytes,
  hint,
  measuring,
  empty,
  progress,
}: {
  label: string;
  /// `null` when nothing has been measured yet, or when the source
  /// could not answer. Never coerced to 0.
  bytes: number | null;
  hint?: string;
  measuring: boolean;
  /// True when the scan ran and found nothing of this kind to size.
  ///
  /// A fourth answer, distinct from a byte count, from "measuring",
  /// and from "not measured". A machine with no virtualenvs is not an
  /// unmeasured machine -- we looked, and there was nothing there --
  /// and leaving it on "Not measured" after a completed pass reports
  /// our own success as a failure to look. This is the one place a
  /// "nothing" is honest, precisely because it answers a question that
  /// was actually asked.
  empty?: boolean;
  /// "3 of 41 repositories" while a batched source is still landing.
  /// The number that makes a partly-filled row legible rather than
  /// looking stuck, the same way `ArtifactsPage` reports it.
  progress?: string;
}) {
  return (
    <div className="flex items-baseline justify-between gap-2 border-t border-[#30363d] py-1.5">
      <span className="text-sm text-[#e6edf3]">
        {label}
        {hint ? <span className="ml-2 text-xs text-[#8b949e]">{hint}</span> : null}
      </span>
      <span className="shrink-0 text-sm tabular-nums">
        {bytes === null ? (
          measuring ? (
            <span className="text-[#8b949e]">Measuring…</span>
          ) : empty ? (
            // We looked and there was nothing. Distinct from both a
            // zero and a "not measured", and the only one of the three
            // that is a real answer.
            <span className="text-[#8b949e]">None found</span>
          ) : (
            <NotMeasured />
          )
        ) : (
          <>
            {/* "at least" while batches are outstanding, for the same
                reason `ArtifactsPage` says it: a total over a partial
                set is not the total, and presenting it as one is wrong
                in the single place the reader is looking. */}
            <span className="text-[#e6edf3]">
              {progress ? "at least " : ""}
              {formatSize(bytes)}
            </span>
            {progress ? (
              <span className="ml-2 text-xs text-[#8b949e]">{progress}</span>
            ) : null}
          </>
        )}
      </span>
    </div>
  );
}

/// The DISK half of the footprint panel, behind an explicit action.
///
/// # Why this is a separate component with its own state
///
/// The four sources here take seconds to tens of seconds --
/// `size_worktrees` alone is ~13s for 147 worktrees, which is #661
/// exactly: a slow command over the remote surface timing out at 120s.
/// So none of them may run because a view opened.
///
/// `measured` is what enforces that, and it is deliberately a piece of
/// state in this component rather than a prop or a hook option: every
/// query below is gated on it, it starts `false`, and the ONLY thing
/// that sets it is the button's onClick. There is no code path that
/// reaches these commands without a click, which is a stronger
/// guarantee than "we remembered to pass enabled: false" -- and it is
/// what the test asserts, because a regression here is invisible on a
/// developer machine with three worktrees and catastrophic on a real
/// one with 147.
///
/// Nothing here re-measures on a timer either. The hooks' own long
/// `staleTime`s (5 minutes for worktrees and artifacts, 30 for venvs)
/// are what make a second visit free, and they are the same cache
/// entries the Worktrees, Artifacts and Docker pages fill -- so opening
/// this after those pages costs nothing, and the numbers agree with
/// them because they ARE them.
///
/// # No sizing code
///
/// Every byte below comes from a command that already exists and is
/// already what another view shows. This component sums; it does not
/// measure. Anything else would be a second implementation of sizing
/// that could disagree with the first, and a user comparing two pages
/// would have no way to tell which was lying.
function DiskFootprint() {
  const [measured, setMeasured] = useState(false);

  // Discovery is gated too, not just sizing. `scan_artifacts` and
  // `scan_venvs` are seconds in their own right (measured: ~1.5s for
  // 178 directories, 9-40s for virtualenvs), so letting them run on
  // mount would reintroduce the cost this panel is avoiding, just in a
  // cheaper-looking place.
  const repos = useWorktrees(measured);
  const artifacts = useArtifacts(measured);
  const venvs = useVenvs(measured);

  const repoPaths = useMemo(() => (repos.data ?? []).map((r) => r.path), [repos.data]);
  const artifactList = useMemo(() => artifacts.data ?? [], [artifacts.data]);
  const venvList = useMemo(() => venvs.data ?? [], [venvs.data]);

  // The same hooks the three pages use, on the same query keys, so this
  // shares their cache rather than racing it.
  const worktreeSizes = useAllWorktreeSizes(repoPaths, measured && repoPaths.length > 0);
  const artifactSizes = useArtifactSizes(artifactList, measured && artifactList.length > 0);
  const venvSizes = useVenvSizes(venvList, measured && venvList.length > 0);
  const docker = useDockerDiskUsage(measured);

  /// Sum a size map, or `null` when nothing has landed.
  ///
  /// The `null` is the point. A map with no entries yet sums to 0, and
  /// returning that would print "0 B" over a measurement still in
  /// flight -- the absent-is-not-zero failure in its most plausible
  /// disguise, because the number is briefly true-looking.
  const sum = (sizes: Map<string, number>): number | null =>
    sizes.size === 0 ? null : [...sizes.values()].reduce((n, v) => n + v, 0);

  const progress = (pending: number, total: number, unit: string) =>
    pending > 0 ? `${total - pending} of ${total} ${unit}` : undefined;

  // Docker's own accounting, not a directory walk: images plus build
  // cache plus volumes, the three figures the Docker page shows. A
  // failed read is `null` and says so -- Docker being off is the
  // ordinary case, and "0 B of images" would claim we asked and it was
  // empty.
  const dockerBytes = docker.data
    ? docker.data.images_bytes +
      docker.data.build_cache_bytes +
      docker.data.volumes_bytes
    : null;

  if (!measured) {
    return (
      <div>
        {/* The cost is stated BEFORE the click, not after. This walks
            every worktree, artifact directory and virtualenv on the
            machine and takes tens of seconds on a real one; a button
            that says only "Measure" and then appears to hang is how a
            user learns to distrust the view. */}
        <p className="text-sm leading-relaxed text-[#8b949e]">
          Headstate can add up the disk its worktrees, build artifacts,
          virtualenvs and Docker images are using. That means walking every one
          of them, which takes tens of seconds on a large machine, so it only
          runs when you ask.
        </p>
        <button
          type="button"
          onClick={() => setMeasured(true)}
          className="mt-3 rounded-md border border-[#30363d] bg-[#21262d] px-3 py-1.5 text-sm text-[#e6edf3] hover:border-[#8b949e]"
        >
          Measure disk use
        </button>
      </div>
    );
  }

  return (
    <div>
      <div className="flex flex-col">
        {/* `empty` is asserted from the DISCOVERY query, not from the
            size map: "the scan succeeded and found no repositories" is
            a fact only discovery knows. An empty size map means merely
            that nothing has landed, which is also true mid-flight. */}
        <DiskRow
          label="Worktrees"
          bytes={sum(worktreeSizes.sizes)}
          measuring={repos.isFetching || worktreeSizes.pending > 0}
          empty={repos.isSuccess && repoPaths.length === 0}
          progress={progress(worktreeSizes.pending, worktreeSizes.total, "repositories")}
        />
        <DiskRow
          label="Build artifacts"
          hint="target/, node_modules/, and the rest"
          bytes={sum(artifactSizes.sizes)}
          measuring={artifacts.isFetching || artifactSizes.pending > 0}
          empty={artifacts.isSuccess && artifactList.length === 0}
          progress={progress(artifactSizes.pending, artifactSizes.total, "repositories")}
        />
        <DiskRow
          label="Virtualenvs"
          bytes={sum(venvSizes.sizes)}
          measuring={venvs.isFetching || venvSizes.measuring}
          empty={venvs.isSuccess && venvList.length === 0}
          progress={progress(venvSizes.pending, venvSizes.total, "batches")}
        />
        <DiskRow
          label="Docker"
          hint="Images, build cache and volumes"
          bytes={dockerBytes}
          measuring={docker.isFetching}
        />
      </div>

      {/* Docker off is not an error and must not be dressed as one --
          most machines do not have it running, and a red row would send
          people to fix something that is not broken. */}
      {docker.isError ? (
        <p className="mt-2 text-xs text-[#8b949e]">
          Docker did not answer, so its disk use is not included. That is
          normal when Docker is not running.
        </p>
      ) : null}

      {/* Where each number came from. The panel summarises the other
          three views rather than measuring anything itself, and saying
          so is what lets a reader who sees a different figure on the
          Worktrees page know which one to trust: they are the same
          number, from the same command, out of the same cache. */}
      <p className="mt-3 text-xs leading-relaxed text-[#8b949e]">
        The same figures the Worktrees, Artifacts and Docker views show, from
        the same measurements — not a second count. Sizes are cached, so those
        views will not re-walk anything you have measured here.
      </p>
    </div>
  );
}

/// The LIVE half: what Headstate's processes cost right now.
///
/// Split from the disk half so the two halves' failure modes stay
/// separate. This one polls and is cheap; that one is manual and slow,
/// and a single component would make it far too easy for a later edit
/// to put a directory walk on a five-second timer.
function LiveFootprint() {
  const fp = useSystemFootprint(true);

  if (fp.isError && fp.data === undefined) {
    return (
      <p className="text-sm text-[#8b949e]">
        Could not read what Headstate is costing: {errorMessage(fp.error)}
      </p>
    );
  }
  const f = fp.data;
  if (!f) {
    return <p className="text-sm text-[#8b949e]">Reading…</p>;
  }

  return (
    <div>
      <div>
        <div className="text-xs text-[#8b949e]">This app</div>
        {f.app === null ? (
          // Not expected on any platform this ships to, but a zero here
          // would read as an idle app rather than as a failed lookup.
          <p className="mt-1 text-sm">
            <NotMeasured>— this machine did not report our own process.</NotMeasured>
          </p>
        ) : (
          <table className="mt-1 w-full text-sm">
            <ProcessHead />
            <tbody>
              <ProcessRow p={f.app} hint="the Headstate window" />
            </tbody>
          </table>
        )}
      </div>

      <div className="mt-4">
        <div className="text-xs text-[#8b949e]">Tools Headstate is running</div>
        {f.children.length === 0 ? (
          // The most important sentence in this panel.
          //
          // An empty list is the ORDINARY state -- `git` runs in bursts
          // and is gone between refreshes -- so it must read as "none
          // right now", not as a row of tools sitting at 0%. A zeroed
          // table here would tell a user their fan-out is idle when it
          // never started, which is the failure the Rust module docs
          // are about and the reason `children` is a list of the living
          // rather than a fixed set of rows.
          <p className="mt-1 text-sm text-[#8b949e]">
            None running at this moment. Headstate starts <code>git</code> and{" "}
            <code>gh</code> in bursts, so this is usually empty between
            refreshes — it does not mean they are idle.
          </p>
        ) : (
          <table className="mt-1 w-full text-sm">
            <ProcessHead />
            <tbody>
              {/* Keyed on the PID, which is what actually identifies a
                  row: several `git` share a name during a scan, and
                  keying on the name would make React reuse one row's
                  DOM for another process. */}
              {f.children.map((c) => (
                <ProcessRow key={c.pid} p={c} />
              ))}
            </tbody>
          </table>
        )}
        {f.children.length > 1 ? (
          <p className="mt-2 text-xs text-[#8b949e]">
            {f.children.length} running at once. A worktree scan starts many{" "}
            <code>git</code> in parallel, which is where a machine that feels
            slow because of Headstate usually is.
          </p>
        ) : null}
      </div>

      <div className="mt-4">
        <div className="text-xs text-[#8b949e]">Docker daemon</div>
        {f.docker_daemon === null ? (
          // Absent, never zero -- and this is the common answer, so it
          // gets a plain sentence rather than a warning colour.
          <p className="mt-1 text-sm text-[#8b949e]">Not running.</p>
        ) : (
          <table className="mt-1 w-full text-sm">
            <ProcessHead />
            <tbody>
              <ProcessRow p={f.docker_daemon} />
            </tbody>
          </table>
        )}
        {/* Why a process we did not start is on this list at all. The
            daemon is not ours and not our child, but Headstate is why
            the user started it, and a daemon holding 4 GB is a cost
            they will attribute to this app -- so it is reported, and
            reported separately so the attribution stays honest. */}
        <p className="mt-2 text-xs leading-relaxed text-[#8b949e]">
          Not started by Headstate and not one of its processes, but its memory
          is a cost the Docker view is usually the reason for — so it is listed
          apart from the two groups above rather than added to them.
        </p>
      </div>
    </div>
  );
}

/// One at-a-glance pressure reading, above the panels.
///
/// # Why a number AND a bar
///
/// The bar answers "roughly how full" at a glance, which is the whole
/// point of this row; the digits answer "how full", which is what
/// someone reaches for the moment the bar looks interesting. Showing
/// only the bar makes the second question require scrolling to a panel,
/// and showing only the number makes the first question require
/// reading. They cost the same together.
///
/// # Colour is never the only cue
///
/// `barColor`'s three bands are the same ones the panels below use, so
/// the row and the detail cannot disagree about what counts as
/// trouble. But the percentage is always rendered as text, so a reader
/// who cannot distinguish the bands loses nothing -- which is what
/// #581/#582 ask for generally, applied here because this row is the
/// most prominent thing on the page.
///
/// # A missing reading is not a zero
///
/// `percent` is `null` when the platform did not report the underlying
/// figure, and this renders "Not measured" with no bar at all. The
/// alternative -- a confident green 0% -- would be the page's own rule
/// broken in the place people look first.
function PressureCard({
  label,
  percent,
  detail,
}: {
  label: string;
  /// `null` when the platform did not report it. Never coerced to 0.
  percent: number | null;
  /// The absolute figures behind the percentage, e.g. "12.4 GB of 16 GB".
  /// A percentage alone cannot distinguish a nearly-full small disk
  /// from a nearly-full large one.
  detail?: string;
}) {
  return (
    <div className="flex flex-col gap-1.5 rounded-md border border-[#30363d] bg-[#0d1117] px-3 py-2.5">
      <span className="text-xs text-[#8b949e]">{label}</span>
      {percent === null ? (
        <>
          <span className="text-lg font-semibold text-[#8b949e]">Not measured</span>
          <div className="h-1.5" />
        </>
      ) : (
        <>
          <span className="text-lg font-semibold tabular-nums text-[#e6edf3]">
            {percent.toFixed(0)}%
          </span>
          <div
            className="h-1.5 overflow-hidden rounded-full bg-[#21262d]"
            role="img"
            aria-label={`${label}: ${percent.toFixed(0)} percent`}
          >
            <div
              className="h-full rounded-full"
              style={{
                width: `${Math.min(100, Math.max(0, percent))}%`,
                backgroundColor: barColor(percent),
              }}
            />
          </div>
        </>
      )}
      {detail ? <span className="text-[11px] text-[#8b949e]">{detail}</span> : null}
    </div>
  );
}

/// One GPU's readings.
///
/// Split out because a machine can have two -- an Intel Mac with
/// integrated and discrete graphics reports both -- and a panel that
/// assumed one would hide whichever it did not pick.
///
/// # Unified memory is stated, not implied
///
/// On Apple Silicon the GPU has no memory of its own: `memory_used` is
/// a share of the same physical pool the Memory panel above reports.
/// Left unsaid, a reader comparing the two panels would add the GPU's
/// gigabytes to the system's and conclude the machine has more RAM than
/// it does -- so the hint on the figure and the note under it both say
/// which pool this is. That is the specific misreading #686 asks to
/// prevent.
function GpuCard({ gpu }: { gpu: HealthGpu }) {
  const memPct =
    gpu.memory_used === null || gpu.memory_total === null
      ? null
      : percentOf(gpu.memory_used, gpu.memory_total);

  return (
    <div className="flex flex-col gap-2">
      <div className="text-sm font-medium text-[#e6edf3]">{gpu.name}</div>
      <div className="flex flex-wrap gap-6">
        {/* Null rather than 0 for a GPU that reported no utilization:
            an idle GPU and an unreadable one are opposite answers, and
            this is the field where they look most alike. */}
        <Stat
          label="Utilization"
          value={
            gpu.utilization_percent === null
              ? null
              : `${gpu.utilization_percent.toFixed(0)}%`
          }
        />
        <Stat
          label="Memory in use"
          value={gpu.memory_used === null ? null : formatSize(gpu.memory_used)}
          hint={gpu.unified_memory && gpu.memory_used !== null ? "Shared with system" : undefined}
        />
        <Stat
          label={gpu.unified_memory ? "Allocated pool" : "Total VRAM"}
          value={gpu.memory_total === null ? null : formatSize(gpu.memory_total)}
        />
      </div>
      {gpu.utilization_percent !== null ? (
        <Bar percent={gpu.utilization_percent} label={`${gpu.name} utilization`} />
      ) : null}
      {memPct !== null ? (
        <div className="text-xs text-[#8b949e]">
          {memPct.toFixed(0)}% of the pool currently allocated to the GPU
        </div>
      ) : null}
      {gpu.unified_memory ? (
        <p className="text-xs leading-relaxed text-[#8b949e]">
          This GPU uses <em>unified memory</em>: it shares one physical pool with
          the CPU rather than having its own. The figures above are a share of
          the same memory the Memory panel reports — not additional memory, so
          they should not be added to it.
        </p>
      ) : null}
    </div>
  );
}

export function SystemHealthPage() {
  // Both queries are enabled unconditionally HERE, because this
  // component only mounts while the view is open -- `App` renders it
  // for `view === "system-health"` and nothing else. That is what
  // stops the polling: the query unmounts, its observer goes, and the
  // interval with it. Gating inside the hook on a prop instead would
  // leave the timer running whenever the caller forgot to pass false.
  const live = useSystemHealth(true);
  const history = useSystemHealthHistory(true);

  // Who this page is about. On the desktop build it is always the
  // machine underfoot, so the connection state is not even consulted --
  // `useConnectionState` is `local` there by construction.
  const connection = useConnectionState();
  const desktopName =
    IS_MOBILE_BUILD && "desktop" in connection ? connection.desktop : null;
  const subject = IS_MOBILE_BUILD ? `${desktopName ?? "the desktop"}'s` : "this machine's";
  const unreachable = IS_MOBILE_BUILD && connection.kind === "unreachable";

  const samples = useMemo(() => history.data ?? [], [history.data]);
  const toPoint = (pick: (s: HealthSample) => number | null): Point[] =>
    samples.map((s) => ({ t: Date.parse(s.sampled_at), v: pick(s) }));

  // Built once per history change rather than inline, so the two charts
  // do not rebuild on every five-second live tick.
  const cpuSeries = useMemo(
    () => toPoint((s) => s.cpu_percent),
    // eslint-disable-next-line react-hooks/exhaustive-deps
    [samples],
  );
  const memSeries = useMemo(
    // Used as a PERCENTAGE of total, not raw bytes: the y axis is then
    // comparable across machines and across a chart whose total never
    // changes anyway.
    () => toPoint((s) => percentOf(s.memory.used, s.memory.total)),
    // eslint-disable-next-line react-hooks/exhaustive-deps
    [samples],
  );
  // The FIRST GPU only. A second one would need its own series, and a
  // machine with two is rare enough that a chart per GPU is not worth
  // the vertical space -- the live figures above already cover both.
  //
  // A sample taken before this shipped, or on a platform that reports
  // no GPU, has no entry and contributes `null`, which `splitOnGaps`
  // breaks the line across. That is the correct reading: the app was
  // running and did not measure a GPU, which is not the same as a GPU
  // that was idle.
  const gpuSeries = useMemo(
    () => toPoint((s) => s.gpus[0]?.utilization_percent ?? null),
    // eslint-disable-next-line react-hooks/exhaustive-deps
    [samples],
  );

  if (live.isError && live.data === undefined) {
    // Health is never served from the companion's cached snapshot the
    // way `get_cached` is -- there is no stored copy of a reading that
    // is only meaningful at the instant it was taken. So on the phone
    // an unreachable desktop arrives here, as a genuine error, and the
    // page says so instead of rendering panels of zeros. A zero in a
    // memory bar is a measurement; "not reachable" is the absence of
    // one, and this codebase does not let the two look alike.
    return (
      <QueryError
        title={
          unreachable
            ? `Cannot reach ${desktopName ?? "the desktop"}`
            : `Could not read ${subject} health`
        }
        message={
          unreachable
            ? "Its health can only be read while it is reachable, and nothing here is stored from before, so there is nothing to show."
            : errorMessage(live.error)
        }
        onRetry={() => void live.refetch()}
      />
    );
  }

  const s = live.data;
  if (!s) {
    // "Not looked yet" and "looked and found nothing" are opposite
    // answers, so the first load says it is loading rather than
    // rendering empty panels full of dashes.
    return (
      <div className="rounded-md border border-[#30363d] px-4 py-12 text-center text-sm text-[#8b949e]">
        Reading {subject} health…
      </div>
    );
  }

  const memUsedPct = percentOf(s.memory.used, s.memory.total);
  const swapPct = percentOf(s.memory.swap_used, s.memory.swap_total);
  // "Now" for everything time-relative on this page: the moment the
  // live sample was taken, which the Rust side stamps. Reading the
  // clock during render instead would make this component impure -- a
  // repaint triggered by anything at all would silently slide the
  // chart's axis and the boot time. Because the live query re-polls
  // every few seconds, this advances on its own anyway.
  const sampledAt = Date.parse(s.sampled_at);

  // The root volume for the pressure card. The Disk panel below still
  // lists every mount -- this row answers "is the machine about to run
  // out", and on every platform we support that question is about the
  // volume the OS is on. `find` rather than a sum across mounts: adding
  // an almost-full external drive to a roomy boot disk would produce a
  // percentage describing no actual volume.
  const rootDisk = s.disks.find((d) => d.is_root) ?? s.disks[0];
  const rootUsed =
    rootDisk === undefined
      ? null
      : percentOf(rootDisk.total - rootDisk.available, rootDisk.total);

  return (
    <div className="flex flex-col gap-4">
      {/* Whose machine, said once, at the top, on the phone only.
          `ConnectionBanner` already names the paired desktop, but it
          is chrome that sits above every view alike -- it says which
          desktop this app is talking to, not what the page under it
          is describing. On a page full of CPU and battery readings
          rendered on a device that has its own CPU and battery, that
          distinction is the whole point, so it is stated on the
          content rather than left to be inferred from the chrome.
          Not repeated per panel: eight panels each captioned with the
          same hostname is noise that stops being read. */}
      {IS_MOBILE_BUILD ? (
        <p className="text-xs text-[#8b949e]">
          Showing the health of{" "}
          <span className="text-[#c9d1d9]">{desktopName ?? "the paired desktop"}</span>, not
          this phone.
        </p>
      ) : null}
      {/* The question people open this page with, answered before
          anything has to be read. Everything below is unchanged; this
          is an addition, not a reorganisation (#683).

          Three across on a phone as well as a desktop: these are short
          numbers, and stacking them would push the panels below the
          fold on exactly the device where a glance matters most. The
          grid handles both widths without a media query, as the panels
          below already do. */}
      <div
        className="grid grid-cols-3 gap-2 sm:gap-4"
        role="group"
        aria-label="System pressure at a glance"
      >
        <PressureCard
          label="CPU"
          percent={s.cpu_percent}
          detail={s.load ? `Load ${s.load[0].toFixed(2)}` : undefined}
        />
        <PressureCard
          label="Memory"
          percent={memUsedPct}
          detail={`${formatSize(s.memory.used)} of ${formatSize(s.memory.total)}`}
        />
        <PressureCard
          label="Disk"
          percent={rootUsed}
          detail={
            rootDisk === undefined
              ? undefined
              : `${formatSize(rootDisk.available)} free of ${formatSize(rootDisk.total)}`
          }
        />
      </div>

      {/* Two columns on a desktop, one on a phone. No media query
          needed: the grid does it, and every panel below is written
          to survive either width. */}
      <div className="grid grid-cols-1 gap-4 md:grid-cols-2">
        <Panel title="CPU" subtitle="Load averages, per-core use, and the last 24 hours">
          <div className="flex flex-wrap gap-6">
            {/* One Stat per window rather than "1.2 / 0.9 / 0.7":
                the three numbers answer different questions (is it
                busy NOW / has it been busy) and a slash-joined triple
                makes people count positions to read one of them. */}
            <Stat
              label="Load (1m)"
              value={s.load ? s.load[0].toFixed(2) : null}
            />
            <Stat label="Load (5m)" value={s.load ? s.load[1].toFixed(2) : null} />
            <Stat label="Load (15m)" value={s.load ? s.load[2].toFixed(2) : null} />
            <Stat
              label="CPU"
              value={s.cpu_percent === null ? null : `${s.cpu_percent.toFixed(0)}%`}
            />
          </div>
          {/* Windows reports no load average at all, so the note is
              shown rather than three silent dashes -- otherwise the
              absence looks like a bug in Headstate. */}
          {s.load === null ? (
            <p className="mt-2 text-xs text-[#8b949e]">
              This platform does not report load averages.
            </p>
          ) : null}

          <div className="mt-4">
            <div className="text-xs text-[#8b949e]">Per core</div>
            {s.cpu_per_core.length === 0 ? (
              <p className="mt-1 text-sm">
                <NotMeasured />
              </p>
            ) : (
              <div className="mt-2 flex flex-col gap-1">
                {s.cpu_per_core.map((v, i) => (
                  <div key={i} className="flex items-center gap-2">
                    <span className="w-10 shrink-0 text-xs tabular-nums text-[#8b949e]">
                      #{i}
                    </span>
                    <Bar percent={v} label={`Core ${i}`} />
                    <span className="w-10 shrink-0 text-right text-xs tabular-nums text-[#8b949e]">
                      {v.toFixed(0)}%
                    </span>
                  </div>
                ))}
              </div>
            )}
          </div>

          <div className="mt-4">
            <div className="text-xs text-[#8b949e]">CPU over the last 24 hours</div>
            <Sparkline points={cpuSeries} max={100} label="CPU" now={sampledAt} />
            <GapNote />
          </div>
        </Panel>

        <Panel title="Memory" subtitle="What is in use, and what the system can still reclaim">
          <div className="flex flex-wrap gap-6">
            <Stat label="Used" value={formatSize(s.memory.used)} />
            {/* "Available" is NOT total minus used, and the hint says
                so: cache counts as used and is also reclaimable, so
                the two numbers do not add up to the total and a reader
                who assumes they do will think one of them is wrong. */}
            <Stat
              label="Available"
              value={formatSize(s.memory.available)}
              hint="Reclaimable, incl. cache"
            />
            <Stat label="Total" value={formatSize(s.memory.total)} />
            <Stat
              label="Pressure"
              value={memUsedPct === null ? null : `${memUsedPct.toFixed(0)}%`}
            />
          </div>
          <div className="mt-3">
            <Bar percent={memUsedPct ?? 0} label="Memory used" />
          </div>

          <div className="mt-4 flex flex-wrap gap-6">
            {/* A machine with swap disabled reports a zero total, which
                is not "0% swap used" -- it is "there is no swap". */}
            <Stat
              label="Swap used"
              value={
                s.memory.swap_total === 0
                  ? null
                  : `${formatSize(s.memory.swap_used)} of ${formatSize(s.memory.swap_total)}`
              }
              hint={s.memory.swap_total === 0 ? "No swap configured" : undefined}
            />
            <Stat
              label="Swap"
              value={swapPct === null ? null : `${swapPct.toFixed(0)}%`}
            />
          </div>

          <div className="mt-4">
            <div className="text-xs text-[#8b949e]">Memory used over the last 24 hours</div>
            <Sparkline
              points={memSeries}
              max={100}
              label="Memory"
              color="#3fb950"
              now={sampledAt}
            />
            <GapNote />
          </div>
        </Panel>

        {/* No GPU panel at all when nothing was discoverable, rather
            than a panel of "Not measured" rows.

            This is the one place on the page where an empty panel
            would be worse than no panel. Elsewhere an absent reading
            sits beside present ones and "Not measured" is informative
            -- this machine has no battery, that platform has no load
            average. A GPU panel containing nothing but absences says
            "we found a GPU and could not read it", which on Windows
            and on Intel/NVIDIA Linux is not what happened: we did not
            look, because there is no unprivileged way to. The issue
            asks for exactly this ("must not render the panel at all if
            there is nothing truthful to put in it").

            On the phone this panel describes the DESKTOP's GPU like
            every other panel here, and needs nothing extra to do so --
            the sample it renders was taken on that machine. */}
        {s.gpus.length > 0 ? (
          <Panel
            title={s.gpus.length === 1 ? "GPU" : "GPUs"}
            subtitle="Utilization and the memory the graphics device is holding"
          >
            <div className="flex flex-col gap-5">
              {s.gpus.map((g, i) => (
                <GpuCard key={`${g.name}-${i}`} gpu={g} />
              ))}
            </div>
            <div className="mt-4">
              <div className="text-xs text-[#8b949e]">
                {s.gpus.length === 1
                  ? "GPU utilization over the last 24 hours"
                  : `${s.gpus[0].name} utilization over the last 24 hours`}
              </div>
              <Sparkline
                points={gpuSeries}
                max={100}
                label="GPU"
                color="#a371f7"
                now={sampledAt}
              />
              <GapNote />
            </div>
          </Panel>
        ) : null}

        <Panel title="Disk" subtitle="Every mounted volume">
          {s.disks.length === 0 ? (
            <p className="text-sm">
              <NotMeasured />
            </p>
          ) : (
            <div className="flex flex-col gap-3">
              {s.disks.map((d) => {
                const used = d.total - d.available;
                const usedPct = percentOf(used, d.total);
                return (
                  <div key={d.mount}>
                    <div className="flex items-baseline justify-between gap-2">
                      <span className="truncate text-sm text-[#e6edf3]">
                        {d.mount}
                        {/* The volume the app lives on is the one a
                            user filling their disk cares about first,
                            so it is named rather than left to be
                            guessed from the mount point. */}
                        {d.is_root ? (
                          <span className="ml-2 rounded bg-[#30363d] px-1.5 py-0.5 text-xs text-[#8b949e]">
                            system
                          </span>
                        ) : null}
                      </span>
                      <span className="shrink-0 text-xs tabular-nums text-[#8b949e]">
                        {formatSize(d.available)} free of {formatSize(d.total)}
                      </span>
                    </div>
                    <div className="mt-1">
                      <Bar percent={usedPct ?? 0} label={`${d.mount} used`} />
                    </div>
                    <div className="mt-1 text-xs tabular-nums text-[#8b949e]">
                      {usedPct === null ? (
                        <NotMeasured />
                      ) : (
                        `${formatSize(used)} used (${usedPct.toFixed(0)}%)`
                      )}
                    </div>
                  </div>
                );
              })}
            </div>
          )}
        </Panel>

        <Panel title="Battery and thermal">
          <div className="flex flex-wrap gap-6">
            {/* A desktop with no battery reports null. "Not measured"
                is right there too: the app did not read a battery,
                and 0% would say the machine is about to die. */}
            <Stat
              label="Battery"
              value={s.battery === null ? null : `${s.battery.percent.toFixed(0)}%`}
              hint={
                s.battery === null
                  ? IS_MOBILE_BUILD
                    ? "That desktop has no battery"
                    : "No battery on this machine"
                  : undefined
              }
            />
            <Stat
              label="Power"
              value={s.battery === null ? null : s.battery.on_ac ? "On AC" : "On battery"}
            />
          </div>
          {s.battery !== null ? (
            <div className="mt-3">
              <Bar percent={s.battery.percent} label="Battery charge" />
            </div>
          ) : null}

          <div className="mt-4">
            <div className="text-xs text-[#8b949e]">Thermal pressure</div>
            <div
              className="text-sm font-medium capitalize"
              style={{ color: s.thermal ? thermalColor(s.thermal) : undefined }}
            >
              {s.thermal ?? <NotMeasured />}
            </div>
            {/* The sentence the issue asks for, and the reason this
                panel is not called "Temperature".

                "Thermal pressure: nominal" invites exactly one wrong
                reading -- that it is a temperature in some unit the
                reader has not spotted. It is not: it is the platform's
                own coarse verdict on how hard it is having to work to
                stay cool. Degrees would need the SMC, which on macOS
                is readable only with elevated privileges
                (`powermetrics --samplers smc` wants sudo), and a
                reading that works only for someone running the app as
                root is worse than an honest qualitative one. Saying so
                costs a line and prevents the misreading. */}
            <p className="mt-2 text-xs leading-relaxed text-[#8b949e]">
              {s.thermal && THERMAL_MEANING[s.thermal]
                ? `${THERMAL_MEANING[s.thermal]} `
                : ""}
              This is the system's own thermal <em>pressure</em> rating, not a
              temperature. Headstate cannot read degrees: that needs elevated
              privileges, so the app reports the label the operating system
              already publishes instead.
            </p>
          </div>
        </Panel>

        <Panel
          title="Network and uptime"
          subtitle="Totals since boot, not rates"
        >
          <div className="flex flex-wrap gap-6">
            <Stat label="Uptime" value={formatUptime(s.uptime_secs)} />
            {/* Derived from uptime rather than read separately, so the
                two can never disagree by a second and look wrong.

                Subtracted from the SAMPLE's timestamp, not from the
                clock at render time: uptime was measured at that
                instant, so pairing it with a later "now" drifts the
                boot time forward on every repaint -- and a boot time
                that moves is obviously wrong to anyone watching it. */}
            <Stat
              label="Booted"
              value={new Date(sampledAt - s.uptime_secs * 1000).toLocaleString()}
            />
          </div>

          <div className="mt-4">
            <div className="text-xs text-[#8b949e]">Interfaces</div>
            {s.networks.length === 0 ? (
              <p className="mt-1 text-sm">
                <NotMeasured />
              </p>
            ) : (
              <table className="mt-2 w-full text-sm">
                <thead>
                  <tr className="text-left text-xs text-[#8b949e]">
                    <th className="font-normal">Interface</th>
                    <th className="font-normal text-right">In</th>
                    <th className="font-normal text-right">Out</th>
                  </tr>
                </thead>
                <tbody>
                  {s.networks.map((n) => (
                    <tr key={n.name} className="border-t border-[#30363d]">
                      <td className="py-1 pr-2 truncate">{n.name}</td>
                      <td className="py-1 text-right tabular-nums text-[#8b949e]">
                        {formatSize(n.rx_bytes)}
                      </td>
                      <td className="py-1 text-right tabular-nums text-[#8b949e]">
                        {formatSize(n.tx_bytes)}
                      </td>
                    </tr>
                  ))}
                </tbody>
              </table>
            )}
            {/* Cumulative since boot is a surprising choice to a reader
                expecting a speed, so it is stated rather than left to
                be inferred from a number that only ever goes up. */}
            <p className="mt-2 text-xs text-[#8b949e]">
              Totals since the machine booted, not current speeds.
            </p>
          </div>
        </Panel>
        {/* Full width, and last.

            The five panels above describe the MACHINE; this one is the
            only panel about Headstate, so it reads as an answer to what
            they raise -- "the machine is busy" then "and here is
            whether we are why". Given both columns because its two
            halves are each a table's width and squeezing them into one
            column would wrap every row.

            The two halves are separate components rather than one:
            the live half polls and is cheap, the disk half is manual
            and takes tens of seconds, and keeping that boundary in the
            component tree is what makes it hard for a later edit to
            put a directory walk on the five-second timer. */}
        <div className="md:col-span-2">
          <Panel
            title="What Headstate is costing"
            subtitle="Its processes right now, and the disk it is using"
          >
            <LiveFootprint />
            <div className="mt-6 border-t border-[#30363d] pt-4">
              <h3 className="text-xs font-semibold text-[#e6edf3]">Disk</h3>
              <div className="mt-2">
                <DiskFootprint />
              </div>
            </div>
          </Panel>
        </div>
      </div>

      {history.isError ? (
        // The live panels above are fine; only the charts are missing.
        // Said in place rather than replacing the page, because the
        // current numbers are the half of this view people open it for.
        <p className="rounded-md border border-[#d29922]/40 bg-[#d29922]/5 px-4 py-2 text-xs text-[#d29922]">
          The 24-hour history could not be loaded, so the charts above are
          empty. The current readings are unaffected.
        </p>
      ) : null}
    </div>
  );
}

/// The one-line explanation of why a chart has holes in it.
///
/// Beside each chart rather than once at the top of the page: a break
/// in a line is noticed while looking at that line, and an explanation
/// two panels away is one nobody reads at the moment they need it.
function GapNote() {
  return (
    <p className="mt-1 text-xs text-[#8b949e]">
      {IS_MOBILE_BUILD
        ? // The phone has a second way to produce a hole that the
          // desktop does not: the desktop can be sampling perfectly
          // while this phone is unable to reach it. Both are honest
          // gaps and neither is interpolated, but they are different
          // facts, and saying only the first would assert the desktop
          // was off during a period it may have been running fine.
          "Breaks in the line are periods with no reading — either Headstate was not running on that desktop, or this phone could not reach it. Nothing is filled in across them."
        : "Breaks in the line are periods when Headstate was not running. Nothing is filled in across them."}
    </p>
  );
}
