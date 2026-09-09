import { useMemo } from "react";
import { useSystemHealth, useSystemHealthHistory } from "@/api/hooks";
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
import type { HealthSample } from "@/types/pr";
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
