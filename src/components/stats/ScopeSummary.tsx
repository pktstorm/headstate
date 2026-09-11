import { Card } from "@/components/ui/card";
import type { AuthorRow, StatsOutcome } from "@/types/pr";
import { LINES_CHANGED_LABEL, REVIEWS_LABEL } from "./Leaderboard";

/// One figure, with whatever qualifies it printed underneath.
///
/// `value` is a string rather than a number so a caller can pass "--" for an
/// unmeasured figure. That is the whole reason it is not a number: the one
/// thing this card must never do is render a `0` for something that was not
/// measured, and a numeric prop makes that the path of least resistance.
function Figure({
  label,
  value,
  hint,
  /// True when the figure could not be measured at all. Styled as absent
  /// rather than as a value, so a failed query does not look like a quiet
  /// week.
  unmeasured = false,
}: {
  label: string;
  value: string;
  hint: string;
  unmeasured?: boolean;
}) {
  return (
    <Card className="px-4">
      <div className="text-xs text-[#8b949e]">{label}</div>
      <div
        className={`mt-1 text-2xl font-semibold tabular-nums ${
          unmeasured ? "text-[#8b949e]" : ""
        }`}
      >
        {value}
      </div>
      <div className="mt-1 text-xs text-[#8b949e]">{hint}</div>
    </Card>
  );
}

/// The headline counts for a scope: merged and opened in the window.
///
/// # A failed count is not a zero
///
/// `merged` and `opened` are independent queries (`useScopedCounts`), and
/// either can fail alone. #826 requires that a failed sub-query be
/// distinguishable from a zero, following `hooks.ts:1397-1434` -- and the
/// reason given there applies exactly: a caller watching only `pending` sees
/// the number fall to zero and concludes everything was measured. So an
/// absent count renders "--" with "could not measure" beneath it, never a 0.
export function ScopeCounts({
  merged,
  opened,
  days,
  failed,
}: {
  merged: StatsOutcome | undefined;
  opened: StatsOutcome | undefined;
  days: number;
  /// How many of the two counts FAILED outright, as opposed to still being
  /// in flight. A pending count shows a skeleton elsewhere; this is for the
  /// ones that came back as errors.
  failed: number;
}) {
  const figure = (o: StatsOutcome | undefined, label: string) => {
    if (!o) {
      return (
        <Figure
          key={label}
          label={label}
          value="--"
          unmeasured
          hint={failed > 0 ? "could not measure" : "measuring..."}
        />
      );
    }
    // The count itself is EXACT even when retrieval was capped -- the
    // 1,000-result limit caps what can be fetched, not what is counted. So
    // the number is presented plainly and the caveat is about the per-PR
    // detail, which is a different claim and belongs on the board.
    const parts = [`last ${days} days`];
    if (o.slices > 1) {
      // Said out loud because an assembled total is a different kind of
      // answer from a single measurement, and #824 item 8 requires anything
      // assembled to say so.
      parts.push(`assembled from ${o.slices} slices`);
    }
    if (o.refusedFields > 0) {
      parts.push(
        `${o.refusedFields} field${o.refusedFields === 1 ? "" : "s"} refused`,
      );
    }
    return (
      <Figure
        key={label}
        label={label}
        value={o.total.toLocaleString()}
        hint={parts.join(" · ")}
      />
    );
  };

  return (
    <div className="grid grid-cols-1 gap-3 sm:grid-cols-2">
      {figure(merged, "Merged")}
      {figure(opened, "Opened")}
    </div>
  );
}

/// One person's figures, for the Mine view.
///
/// `row` is `undefined` when that person has no activity in the window, and
/// that renders as "no activity" rather than as four zeroes. #826's rule:
/// "empty means empty -- a member with no activity in the window reads as
/// 'no activity', not a zero that might be a failed query". The distinction
/// is only available because the Rust `Board::row_for` returns `None` rather
/// than a zero row.
export function PersonFigures({
  row,
  who,
  /// True when the BOARD was incomplete. A person's figures computed from a
  /// partial board are floors, not totals, and saying so is the difference
  /// between a number and a claim.
  partial,
}: {
  row: AuthorRow | undefined;
  /// Whose figures these are, for the empty state's wording. "You" reads
  /// better than a login when it is the viewer.
  who: string;
  partial: boolean;
}) {
  if (!row) {
    // An ABSENT row means two completely different things, and which one it
    // means depends on `partial` rather than on anything visible in the row
    // itself. Either the person genuinely merged nothing, or the slice that
    // held their pull requests came back short or refused and their row was
    // never built.
    //
    // So "that is a measured result, not a missing one" is only sayable on a
    // COMPLETE board. Said over a partial one it is the #802/#790 confusion
    // inverted -- not a zero that might be a failure, but an explicit denial
    // that it could be one, which is worse because it is the sentence a
    // reader would rely on.
    return (
      <div className="rounded-md border border-[#30363d] px-4 py-10 text-center">
        <p className="text-sm font-semibold text-[#e6edf3]">
          {partial ? "No activity found" : "No activity"}
        </p>
        <p className="mx-auto mt-2 max-w-md text-sm text-[#8b949e]">
          {partial ? (
            <>
              Nothing was found for {who} in this window and scope -- but this
              board is incomplete, so a missing row may be missing data rather
              than absent work. See the note above.
            </>
          ) : (
            <>
              {who} opened or merged no pull requests in this window and this
              scope. That is a measured result, not a missing one.
            </>
          )}
        </p>
      </div>
    );
  }
  // "at least" when the board is partial. A floor presented as a total is
  // the exact defect #802 and #790 shipped, and the prefix is the cheapest
  // honest fix at a figure whose denominator the reader cannot see.
  const at = partial ? "at least " : "";
  return (
    <div className="grid grid-cols-1 gap-3 sm:grid-cols-2 lg:grid-cols-4">
      <Figure
        label="Pull requests"
        value={`${at}${row.prs.toLocaleString()}`}
        hint="in this window and scope"
      />
      <Figure
        label="Lines changed"
        value={`${at}${(row.additions + row.deletions).toLocaleString()}`}
        // The honest label, every time the measure appears. #823 settled
        // the label AS the mitigation for a gameable metric, so it is not
        // dropped for space on a narrower card.
        hint={LINES_CHANGED_LABEL}
      />
      <Figure
        label="Files touched"
        value={`${at}${row.changedFiles.toLocaleString()}`}
        hint="the companion to lines changed"
      />
      <Figure
        label="Reviews"
        value={`${at}${row.reviewsReceived.toLocaleString()}`}
        hint={REVIEWS_LABEL}
      />
    </div>
  );
}

/// Everybody else in scope, aggregated.
///
/// The "Others" counterpart to `PersonFigures`: the same four measures over
/// the rest of the population, so the two views are comparable rather than
/// one being a table and the other four cards.
export function GroupFigures({
  rows,
  partial,
}: {
  rows: AuthorRow[];
  partial: boolean;
}) {
  if (rows.length === 0) {
    // The same two-meanings problem `PersonFigures` has, one level up: an
    // empty population is a measured fact on a complete board and an open
    // question on a partial one. "That is an absence of pull requests, not an
    // absence of people" is a claim, and it must not be made over data that
    // came back short.
    return (
      <div className="rounded-md border border-[#30363d] px-4 py-10 text-center">
        <p className="text-sm font-semibold text-[#e6edf3]">
          {partial ? "Nobody else found" : "Nobody else in this window"}
        </p>
        <p className="mx-auto mt-2 max-w-md text-sm text-[#8b949e]">
          {partial ? (
            <>
              No other author was found in this scope and window -- but this
              board is incomplete, so there may be people whose pull requests
              were not retrieved. See the note below.
            </>
          ) : (
            <>
              No other author opened or merged a pull request in this scope and
              window. A member with no activity has no row here -- that is an
              absence of pull requests, not an absence of people.
            </>
          )}
        </p>
      </div>
    );
  }
  const sum = (f: (r: AuthorRow) => number) => rows.reduce((t, r) => t + f(r), 0);
  const at = partial ? "at least " : "";
  return (
    <div className="grid grid-cols-1 gap-3 sm:grid-cols-2 lg:grid-cols-4">
      <Figure
        label="People"
        value={rows.length.toLocaleString()}
        // Counted from rows, so it is people WITH ACTIVITY rather than
        // members of the org. Said plainly, because "People: 3" under an
        // org with twelve members would otherwise look like a failed roster
        // fetch.
        hint="with activity in this window"
      />
      <Figure
        label="Pull requests"
        value={`${at}${sum((r) => r.prs).toLocaleString()}`}
        hint="across everyone else"
      />
      <Figure
        label="Lines changed"
        value={`${at}${sum((r) => r.additions + r.deletions).toLocaleString()}`}
        hint={LINES_CHANGED_LABEL}
      />
      <Figure
        label="Files touched"
        value={`${at}${sum((r) => r.changedFiles).toLocaleString()}`}
        hint="the companion to lines changed"
      />
    </div>
  );
}
