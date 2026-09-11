import { Card } from "@/components/ui/card";
import type { AuthorRow } from "@/types/pr";

/// How many rows a leaderboard shows.
///
/// Must equal the Rust `board::TOP_N`, which is what actually cuts the list
/// -- this is the number the HEADING quotes. Re-cutting here would produce a
/// "top five" heading over three rows, or a top-three whose fourth place was
/// decided by a tie-break this side never saw.
export const TOP_N = 5;

/// The label for the lines-changed measure, and it is load-bearing.
///
/// #823 settled the code metric as raw `additions`/`deletions` with the
/// honest label AS the mitigation: "this metric is gameable by a large
/// generated diff, and people who know they are ranked will notice. The
/// honest label is the mitigation, not a fix."
///
/// So the words "including generated files" are not decoration and must not
/// be shortened for layout. A reader who takes this for a measure of effort
/// has been misled by the chart, and there is nothing else on screen to
/// correct them. Exported so the one phrasing is reused wherever the measure
/// appears rather than re-worded per component.
export const LINES_CHANGED_LABEL = "lines changed, including generated files";

/// What the review count actually measures.
///
/// RECEIVED, not given. It reads `reviews { totalCount }` off a pull request
/// the author WROTE, so it counts how much review their work attracted --
/// close to the opposite of what "top reviewers" would suggest to a reader
/// skimming. A board of reviews GIVEN needs `reviewed-by:<login>`, which is
/// one search per person and a different question entirely; until that is
/// built the label has to carry the distinction.
export const REVIEWS_LABEL = "reviews received on their pull requests";

/// One ranked list, as horizontal bars.
///
/// Bars rather than a recharts chart, deliberately, and not for lack of the
/// library: `ActivityChart` uses recharts because a 30-point time series
/// needs axes, a tooltip and interpolation. A five-row ranking needs a label,
/// a proportion and a number, which is exactly what `RepoTable` already draws
/// with a span and a width percentage -- so this reuses THAT idiom. #826 asks
/// that the existing components be reused rather than a second charting idiom
/// introduced, and the second idiom would have been a recharts bar chart
/// beside a hand-drawn bar table, not a chart beside no chart.
///
/// # The bar is a share of the LEADER, not of the total
///
/// A share of the total would make every bar short as soon as the population
/// is large -- in a 40-person org the leader's bar would be 8% wide and the
/// ranking unreadable. Against the leader, the shape answers the question a
/// ranking is for: how far ahead is first place. The numbers are printed
/// beside the bars so the absolute figures are never only a bar length.
function Ranked({
  title,
  hint,
  rows,
  value,
  format,
  emptyNote,
}: {
  title: string;
  /// What the measure IS. Never optional: every measure here is either
  /// gameable (lines changed) or easy to misread (reviews received), and
  /// the hint is where that is said.
  hint: string;
  rows: AuthorRow[];
  value: (r: AuthorRow) => number;
  format: (r: AuthorRow) => string;
  /// What to say when nobody qualifies. Distinct per measure, because "no
  /// pull requests" and "no lines changed" are different facts.
  emptyNote: string;
}) {
  // The LEADER's value, which is the bar scale. Taken from the rows rather
  // than assumed to be the first, so this is correct even if a caller hands
  // over an unsorted list.
  const leader = rows.reduce((m, r) => Math.max(m, value(r)), 0);

  return (
    <Card className="px-4">
      <div className="text-sm font-semibold">{title}</div>
      <div className="text-xs text-[#8b949e]">{hint}</div>
      {rows.length === 0 ? (
        <div className="py-8 text-center text-sm text-[#8b949e]">{emptyNote}</div>
      ) : (
        <ol className="mt-3 flex flex-col gap-1">
          {rows.map((r, i) => {
            const pct = leader === 0 ? 0 : Math.round((value(r) / leader) * 100);
            return (
              <li
                key={r.login}
                className="flex items-center gap-3 rounded px-2 py-1.5 text-sm"
              >
                {/* The rank as a number. A ranking whose order is carried
                    only by vertical position is unreadable to anyone
                    hearing it read out, and `ol` markup alone does not
                    surface the index in most screen readers. */}
                <span className="w-4 shrink-0 text-right text-xs tabular-nums text-[#8b949e]">
                  {i + 1}
                </span>
                <span className="w-40 shrink-0 truncate text-left" title={r.login}>
                  {r.login}
                </span>
                <span
                  className="relative h-1.5 flex-1 overflow-hidden rounded bg-[#21262d]"
                  // The bar is decoration over a number that is already
                  // printed beside it, so it is hidden rather than given an
                  // ARIA value that would read the same figure twice.
                  aria-hidden="true"
                >
                  <span
                    className="absolute inset-y-0 left-0 rounded bg-[#58a6ff]"
                    style={{ width: `${pct}%` }}
                  />
                </span>
                <span className="w-28 shrink-0 text-right tabular-nums text-xs">
                  {format(r)}
                </span>
              </li>
            );
          })}
        </ol>
      )}
    </Card>
  );
}

/// The three leaderboards #826 asks for: top authors, top reviewers, top by
/// code volume.
///
/// # Everyone who can open a scope sees this
///
/// No role gating, which #823 settled with its consequence recorded rather
/// than assumed away: "this publishes a peer-visible ranking of colleagues,
/// not just a lead-facing one". It is repeated here because this component is
/// where that decision becomes visible, and a future reader wondering whether
/// the missing permission check is an oversight should find the answer at the
/// code rather than in an issue thread.
///
/// # Ranking over a partial board
///
/// A ranking is far less forgiving of missing data than a count: a total 5%
/// short is slightly wrong, while a top-five 5% short can have the wrong
/// person in first place. So `complete` is a REQUIRED prop and a partial
/// board renders the caveat above the boards rather than beside one of them
/// -- it applies to all three, and #826's rule is "never a confident
/// top-five over a sample".
export function Leaderboards({
  rows,
  complete,
  caveat,
}: {
  /// Every author in scope. Ranked and cut here, per measure, because the
  /// three rankings disagree -- the most prolific author is rarely the one
  /// with the most lines.
  rows: AuthorRow[];
  complete: boolean;
  /// Why the board is partial, in the caller's words -- the caller knows
  /// which of the three partiality channels applied and this component does
  /// not.
  ///
  /// OPTIONAL, because `StatsPage` carries the detail in a page-level banner
  /// that also covers the Mine view's figures, and two copies of one warning
  /// read as two different problems. Omitted, the short reminder below still
  /// renders: a reader taking a name off a ranking needs the caveat where
  /// their eye is, not only at the top of the page.
  caveat?: string;
}) {
  const top = (value: (r: AuthorRow) => number) =>
    [...rows]
      // A zero has no rank. Padding a top-five with zeroes presents people
      // as ranked on a measure they do not appear in at all -- which for
      // "top reviewers" would list colleagues as reviewed when nobody
      // reviewed them.
      .filter((r) => value(r) > 0)
      // Ties break on LOGIN so the boards do not reorder between loads when
      // nothing changed. The Rust side sorts the same way for the same
      // reason; doing it here too means a caller that re-sorts locally
      // cannot reintroduce the flicker.
      .sort((a, b) => value(b) - value(a) || a.login.localeCompare(b.login))
      .slice(0, TOP_N);

  return (
    <div className="flex flex-col gap-3">
      {!complete ? (
        <div className="rounded-md border border-[#d29922]/40 bg-[#d29922]/10 px-3 py-2 text-xs text-[#d29922]">
          {/* Above the boards, not inside one: the partiality applies to
              every ranking below it, and a note attached to one board would
              read as though the others were complete.

              Rendered on `!complete` ALONE, with or without a reason. An
              unexplained warning is worth far more than a silent confident
              top-five, and the reason is optional precisely because the page
              may be carrying it elsewhere. */}
          These rankings are incomplete{caveat ? `. ${caveat}` : ", so the order may be wrong."}
        </div>
      ) : null}
      <div className="grid grid-cols-1 gap-3 lg:grid-cols-3">
        <Ranked
          title={`Top ${TOP_N} pull request authors`}
          hint="pull requests in this window"
          rows={top((r) => r.prs)}
          value={(r) => r.prs}
          format={(r) => `${r.prs.toLocaleString()} PRs`}
          emptyNote="No pull requests in this window."
        />
        <Ranked
          title={`Top ${TOP_N} by code volume`}
          hint={LINES_CHANGED_LABEL}
          rows={top((r) => r.additions + r.deletions)}
          value={(r) => r.additions + r.deletions}
          // The file count rides along, because it is what distinguishes a
          // generated diff from a refactor and it is a free scalar. Without
          // it the measure's one honest defence is a sentence nobody reads
          // twice; with it the reader can see 40,000 lines in 3 files for
          // themselves.
          format={(r) =>
            `${(r.additions + r.deletions).toLocaleString()} · ${r.changedFiles.toLocaleString()} files`
          }
          emptyNote="No lines changed in this window."
        />
        {/* NOT titled "Top reviewers", which is what #826 asked for and
            what the data cannot support. `reviews { totalCount }` hangs off
            a pull request the author WROTE, so it counts review their work
            ATTRACTED -- close to the opposite of the reading "top
            reviewers" invites, and the person at the top of a board so
            titled would be the one whose code was reviewed most, not the
            one who reviewed most.

            A board of reviews GIVEN is a real and answerable question
            (`reviewed-by:<login>`), and it is one search PER PERSON -- a
            fan-out over the org's whole membership, which is a different
            cost class from this document's free scalar and needs its own
            measurement. Recorded as a deliberate narrowing rather than
            shipped under the requested title, because a correctly-titled
            board of a different measure is the one version of this that
            cannot mislead. */}
        <Ranked
          title={`Top ${TOP_N} most-reviewed`}
          hint={REVIEWS_LABEL}
          rows={top((r) => r.reviewsReceived)}
          value={(r) => r.reviewsReceived}
          format={(r) => `${r.reviewsReceived.toLocaleString()} reviews`}
          emptyNote="No reviews in this window."
        />
      </div>
    </div>
  );
}
