import { useState } from "react";
import {
  type StatsScope,
  scopeIsLoadable,
  useScopedCounts,
  useStatsBoard,
  useStatsSeries,
} from "../api/hooks";
import { useActiveFilters } from "../store/filters";
import type { ShortSlice } from "../types/pr";
import { QueryError, errorMessage } from "./QueryError";
import { ActivityChart } from "./stats/ActivityChart";
import { CycleTime } from "./stats/CycleTime";
import { HelpButton } from "./HelpButton";
import { Leaderboards } from "./stats/Leaderboard";
import { Outliers } from "./stats/Outliers";
import { RepoTable } from "./stats/RepoTable";
import { GroupFigures, PersonFigures, ScopeCounts } from "./stats/ScopeSummary";
import { SkeletonChart, SkeletonRow } from "./stats/Skeleton";

/// The windows a scope page offers, in days.
///
/// The same three `ActivityChart` already offers, so the chart's own range
/// buttons and the page's window are one control rather than two that can
/// disagree about what "this period" means. 30 is the default because it is
/// the shortest window in which a monthly cadence of work is visible at all,
/// and it is what the unscoped page defaulted to.
const RANGES = [7, 14, 30];

/// Which half of a scope the page is showing.
///
/// Two views, not two pages: they are the same measurement partitioned, so a
/// switch between them costs nothing -- the board is already loaded and
/// `row_for` / `others` split it. That is why this is local state and not a
/// persisted filter.
type Half = "mine" | "others";

/// The PR Stats view: two views on every scope, and the leaderboards.
///
/// # Progressive rendering, extended rather than inherited
///
/// The page before this ran three independent queries, each rendering the
/// moment IT landed rather than behind one combined gate, because they
/// differed enough in cost that a single gate wasted most of the wait
/// (periods ~1.6s, the daily series ~3.7s, the merged sample ~3.7s).
///
/// That property is kept and it matters MORE here, which #826 says
/// explicitly: an "Others" view over an organisation has more parts and more
/// variance than the account-wide page did. The parts now are
///
///   - two scoped counts, merged and opened (count-only, fastest);
///   - the daily series (count-only, ~1.4-1.5s per ten days MEASURED);
///   - the board (per-PR nodes across every slice -- seconds on a busy org,
///     and the only part whose cost scales with how much work happened).
///
/// Each renders as it arrives. A single gate would hide two sub-second
/// answers behind the one that is inherently slow.
///
/// # Nothing loads until clicked
///
/// `enabled` is threaded into all three hooks from one place: whether a scope
/// is actually selected. Arriving at the view with nothing clicked costs
/// nothing beyond the sidebar's own 2 points, which is
/// `hooks.ts:712-717`'s discovery/measurement split. The old "Measure
/// button" pattern is gone -- #796 removed the last one and
/// `SystemHealthPage.tsx:1763-1788` argues against re-adding one -- so the
/// gate is the selection, and the sidebar row is the click that opens it.
export function StatsPage() {
  const [days, setDays] = useState(30);
  const [half, setHalf] = useState<Half>("mine");
  const filters = useActiveFilters();

  // The selection the sidebar wrote, as one object. The three keys ARE one
  // selection (`setStatsScope`), so they are read together and passed
  // together rather than threaded as three arguments that could drift apart.
  const scope: StatsScope | undefined = filters.statsScopeKind
    ? {
        kind: filters.statsScopeKind,
        value: filters.statsScopeValue,
        subject: filters.statsSubject,
      }
    : undefined;
  const loadable = scopeIsLoadable(scope);

  const counts = useScopedCounts(scope, days, loadable);
  const seriesQ = useStatsSeries(scope, days, loadable);
  // Merged, not opened: the board's measures are about work DELIVERED, and
  // a leaderboard of opened pull requests would rank people on intake. The
  // opened count still appears in the headline figures, where it is the
  // intake half of the pair.
  const boardQ = useStatsBoard(scope, "merged", days, loadable);

  const board = boardQ.data;
  const series = seriesQ.data;

  // Nothing selected. Not an error and not a loading state -- the user has
  // simply not asked a question yet, and the sidebar is where they ask it.
  if (!loadable) {
    return (
      <div className="rounded-md border border-[#30363d] px-4 py-12 text-center">
        <p className="text-sm font-semibold text-[#e6edf3]">
          Pick something to measure
        </p>
        <p className="mx-auto mt-2 max-w-md text-sm text-[#8b949e]">
          Choose an organization, a repository or a person in the sidebar.
          Nothing is measured until you do -- a scope-wide load costs rate
          limit, so it waits for a click.
        </p>
      </div>
    );
  }

  // Every part failed. Without this branch each section shows its own error
  // and the page becomes three copies of the same message -- and the cause
  // is usually one thing (no token, no network, budget exhausted) rather
  // than three.
  const allFailed = counts.failed === 2 && seriesQ.isError && boardQ.isError;
  const retryAll = () => {
    counts.refetch();
    void seriesQ.refetch();
    void boardQ.refetch();
  };
  if (allFailed) {
    return (
      <QueryError
        title="Could not load statistics for this scope"
        message={errorMessage(boardQ.error)}
        onRetry={retryAll}
      />
    );
  }

  const scopeLabel = describeScope(scope);
  const subject = scope.subject;
  // Who "Mine" is about. A Members row sets a subject and keeps the org
  // scope, so on that selection "Mine" is that COLLEAGUE rather than the
  // viewer -- which is the question the row asks ("this person, in this
  // org"). `board.viewer` travels with the board so this split cannot be
  // made against a login from a different account.
  const mineLogin = subject ?? board?.viewer;
  const mineRow = board && mineLogin ? board.rows.find((r) => r.login === mineLogin) : undefined;
  const otherRows = board && mineLogin ? board.rows.filter((r) => r.login !== mineLogin) : [];

  // Why the board is partial, assembled from whichever channels applied.
  // All three are reported, because they fail for different reasons and a
  // reader deciding whether to trust a ranking needs to know which.
  const caveat = board ? partialityCaveat(board) : undefined;

  return (
    <div className="flex flex-col gap-3">
      <div className="flex flex-wrap items-center justify-between gap-2">
        <div className="flex items-center gap-1 text-xs text-[#8b949e]">
          <span>{scopeLabel}</span>
          <HelpButton topic="stats-sample" />
        </div>
        <div className="flex gap-1">
          {RANGES.map((r) => (
            <button
              key={r}
              type="button"
              aria-pressed={days === r}
              onClick={() => setDays(r)}
              className={`rounded px-2 py-1 text-xs ${
                days === r
                  ? "bg-[#1f6feb] text-white"
                  : "text-[#8b949e] hover:bg-[#161b22]"
              }`}
            >
              {r}d
            </button>
          ))}
        </div>
      </div>

      {/* The headline counts land first and are the cheapest part. They are
          scope-wide rather than per-view, because "how much happened here"
          is the same question whichever half you are reading. */}
      {counts.merged || counts.opened || counts.failed > 0 ? (
        <ScopeCounts
          merged={counts.merged}
          opened={counts.opened}
          days={days}
          failed={counts.failed}
        />
      ) : (
        <SkeletonRow count={2} cols="sm:grid-cols-2" />
      )}

      {series ? (
        <>
          <ActivityChart
            // The scoped series carries the same three fields
            // `HistoryPoint` does, so it renders through the SAME chart --
            // #826 asks that the existing components be reused rather than
            // a second charting idiom introduced.
            points={series.points}
            days={days}
            onDaysChange={setDays}
          />
          {/* Named days, not a count. A chart of 30 days missing 2 is still
              the most informative thing available, provided it says which 2
              -- and a missing day rendered as zero would draw a trough that
              reads as a quiet Tuesday. */}
          {series.failedDays.length > 0 && (
            <p className="text-xs text-[#d29922]">
              {series.failedDays.length} day
              {series.failedDays.length === 1 ? "" : "s"} could not be measured
              and {series.failedDays.length === 1 ? "is" : "are"} absent from
              the chart rather than drawn as zero:{" "}
              {series.failedDays.join(", ")}.
            </p>
          )}
        </>
      ) : seriesQ.isError ? (
        <QueryError
          title="Could not load the activity chart"
          message={errorMessage(seriesQ.error)}
          onRetry={() => void seriesQ.refetch()}
        />
      ) : (
        <SkeletonChart
          title="Pull request activity"
          hint="Opened and merged per day"
        />
      )}

      {/* The two views. Rendered as a switch rather than two pages because
          they are one measurement partitioned -- switching costs nothing,
          since the board is already loaded. */}
      <div
        className="flex gap-1 border-b border-[#30363d]"
        role="tablist"
        aria-label="Which half of this scope"
      >
        {(
          [
            ["mine", subject ? subject : "Mine"],
            ["others", "Others"],
          ] as const
        ).map(([id, label]) => (
          <button
            key={id}
            type="button"
            role="tab"
            aria-selected={half === id}
            onClick={() => setHalf(id as Half)}
            className={`-mb-px border-b-2 px-3 py-1.5 text-sm ${
              half === id
                ? "border-[#1f6feb] text-[#e6edf3]"
                : "border-transparent text-[#8b949e] hover:text-[#e6edf3]"
            }`}
          >
            {label}
          </button>
        ))}
      </div>

      {board ? (
        <>
          {/* The partiality caveat sits ABOVE both views rather than inside
              the leaderboards, because it qualifies every figure drawn from
              the board -- Mine's four cards, the cycle-time distribution, the
              outliers and the repo shares included, not just the rankings.

              It was inside `Leaderboards` first, which left the Mine tab
              saying "at least 12" with nothing anywhere on screen to say WHY
              it was a floor. A reader cannot act on a prefix alone. */}
          {!board.complete && caveat ? (
            <div className="rounded-md border border-[#d29922]/40 bg-[#d29922]/10 px-3 py-2 text-xs text-[#d29922]">
              This board is incomplete, so every figure below is a floor rather
              than a total. {caveat}
            </div>
          ) : null}
          {half === "mine" ? (
            <>
              <PersonFigures
                row={mineRow}
                who={subject ? subject : "You"}
                partial={!board.complete}
              />
              {/* Cycle time, the outliers and the repo table -- the three
                  sections the unscoped page had and #826 keeps for Mine.
                  Drawn from the SAME board as the figures above, so they
                  cannot disagree with them; the unscoped page fetched them
                  from a separate `get_merged_detail` sample, which is why it
                  needed a page-level "from a sample of recent merged pull
                  requests" caveat that a scope page does not.

                  Rendered only when there is a row: with no activity there is
                  no distribution, and three empty cards under "No activity"
                  would be three more ways to say the same thing. */}
              {mineRow ? (
                <>
                  <CycleTime hours={mineRow.cycleTimeHours} prs={mineRow.prs} />
                  <Outliers
                    // The outliers are the SCOPE's, not this person's, and
                    // that is deliberate on the Mine view too: "the slowest
                    // pull request here" is the useful question, and
                    // narrowing it to one author on a single-author scope
                    // would produce the identical list under a narrower
                    // claim. The rows name their author, so whose they are
                    // is never in doubt.
                    slowest={board.slowest}
                    largest={board.largest}
                    slowestBy={(pr) => pr.cycleTimeHours}
                    hint={
                      board.complete
                        ? "across everyone in this scope and window"
                        : "across the part of this scope that could be measured"
                    }
                  />
                  <RepoTable
                    repos={board.repoCounts}
                    // NOT `sampleSize`. These shares are of the whole
                    // window, not of a fixed recent sample, so the
                    // component's default wording would understate a
                    // complete measurement -- and when it is not complete
                    // the caveat above the leaderboards says why.
                    hint={
                      board.complete
                        ? `share of all ${board.total.toLocaleString()} merged in this window`
                        : `share of the ${board.retrieved.toLocaleString()} of ${board.total.toLocaleString()} merged that could be measured`
                    }
                  />
                </>
              ) : null}
            </>
          ) : (
            <>
              <GroupFigures rows={otherRows} partial={!board.complete} />
              {/* Ranked over EVERYONE including the viewer, not over
                  `otherRows`. A leaderboard that silently excluded the
                  reader would put whoever is second in first place, which
                  is a wrong ranking rather than a filtered one -- and the
                  reader is the one person who can tell it is wrong. */}
              {/* No `caveat` here: the page-level banner above already
                  carries it, and two copies of the same warning reads as two
                  different problems. `complete` is still passed, because the
                  component's own short reminder on the rankings is where a
                  reader's eye actually is when they read a name off a
                  board. */}
              <Leaderboards rows={board.rows} complete={board.complete} />
            </>
          )}
        </>
      ) : boardQ.isError ? (
        <QueryError
          title="Could not load this scope's people"
          message={errorMessage(boardQ.error)}
          onRetry={() => void boardQ.refetch()}
        />
      ) : (
        <SkeletonRow count={4} cols="sm:grid-cols-2 lg:grid-cols-4" />
      )}
    </div>
  );
}

/// One line naming what is being measured.
///
/// Spelled out because every figure on the page is relative to it, and a
/// page that silently changed scope when the sidebar was clicked would
/// present one organisation's numbers under another's -- which nothing else
/// on screen would contradict.
export function describeScope(scope: StatsScope): string {
  const where =
    scope.kind === "repo"
      ? scope.value
      : scope.kind === "org"
        ? `everything in ${scope.value}`
        : scope.kind === "user"
          ? `${scope.value}'s own repositories`
          : "everything this token can see";
  // The subject, when there is one, KEEPS the scope -- "this person, in this
  // org" is the question a Members row asks, so both halves are named.
  return scope.subject ? `${scope.subject}, in ${where}` : String(where);
}

/// Why a board is partial, in words a reader can act on.
///
/// Assembled from all three channels rather than reporting the first, and
/// that is the point of their being separate fields: they fail for different
/// reasons, and "some slices were short" and "GitHub refused fields" suggest
/// different things to do about it. Returns `undefined` for a complete board
/// so the caller has nothing to render.
export function partialityCaveat(board: {
  complete: boolean;
  total: number;
  retrieved: number;
  // The named type rather than an inline shape, so a field added to it on
  // the Rust side reaches this function's reader rather than being silently
  // absent from a structural duplicate.
  truncatedSlices: ShortSlice[];
  refusedFields: number;
}): string | undefined {
  if (board.complete) return undefined;
  const parts: string[] = [];
  if (board.retrieved < board.total) {
    // The SIZE of the gap, not just its existence. A reader deciding whether
    // a top-five is trustworthy needs to know whether four pull requests are
    // missing or four hundred.
    parts.push(
      `${(board.total - board.retrieved).toLocaleString()} of ${board.total.toLocaleString()} pull requests could not be retrieved`,
    );
  }
  if (board.truncatedSlices.length > 0) {
    parts.push(
      `${board.truncatedSlices.length} date range${
        board.truncatedSlices.length === 1 ? "" : "s"
      } came back short`,
    );
  }
  if (board.refusedFields > 0) {
    parts.push(
      `GitHub refused ${board.refusedFields} field${
        board.refusedFields === 1 ? "" : "s"
      } -- if this organization uses SAML single sign-on, authorize your token for it`,
    );
  }
  // A board can be incomplete with none of the above: an irreducible slice
  // is over the 1,000-result cap before any request is made, which the Rust
  // side folds into `complete` directly. Saying so generically beats saying
  // nothing, which would leave "These rankings are incomplete." with no
  // reason attached.
  if (parts.length === 0) {
    parts.push("part of this window holds more pull requests than GitHub will return");
  }
  return `${parts.join("; ")}.`;
}
