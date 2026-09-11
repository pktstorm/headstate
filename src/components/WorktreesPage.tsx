import { ActingOnDesktop } from "./ActingOnDesktop";
import { summarisePull } from "@/lib/pullSummary";
import { ExternalLink } from "./ExternalLink";
import { Sparkles } from "lucide-react";
import { useMemo, useState } from "react";
import {
  useClearAssessed,
  useMarkAssessed,
  useRemoveWorktree,
  useAssessed,
  usePullRequests,
  useRemoveWorktreeForced,
  useRemoveWorktrees,
  useWorktreeSafety,
  useWorktreeSizes,
  useWorktrees,
  usePullCheckout,
  useRemoveOrphan,
  useAllWorktreeSizes,
  useRemovalProgress,
  useAssessment,
  useUnlockWorktree,
  useUnlockWorktrees,
  usePruneWorktrees,
} from "../api/hooks";
import {
  formatSize,
  totalSize,
  canClaudify,
  isPending,
  isSafe,
  pathBasename,
  prForWorktree,
  worktreeSignal,
  isOrphaned,
  ORPHAN_FILTER,
  forceWarning,
  safetyReason,
  safetyTone,
  upstreamReason,
  upstreamShort,
  upstreamTone,
  refAge,
  lockAge,
  lockHolderNote,
  isDeadLock,
  sortWorktrees,
  WORKTREE_SORT_LABELS,
  type WorktreeSort,
} from "../lib/worktrees";
import { HelpButton } from "./HelpButton";
import { WorktreeKebab } from "./WorktreeKebab";
import { claudifyCommand } from "../api/tauri";
import { IS_MOBILE_BUILD } from "@/lib/target";
import { isCancelled } from "@/lib/cancelled";
import { copyText } from "../lib/clipboard";
import { relativeTime } from "../lib/time";
import { useIsMobile } from "../lib/useIsMobile";
import { assessmentSummary } from "../lib/assessment";
import { rollupRepos } from "../lib/rollup";
import { useActiveFilters, useFilters } from "../store/filters";
import type { PullRequest, Worktree } from "../types/pr";
import { toast } from "sonner";
import { QueryError, errorMessage } from "./QueryError";
import { Button } from "./ui/button";
import { Dialog, DialogContent, DialogTitle } from "./ui/dialog";

/// Local git worktrees, so lingering ones can be found and removed.
///
/// Sorted largest-first by default -- with 152 worktrees on one
/// repository, the biggest offenders are what you came for -- and by
/// name, size or age on request (#771). But SAFETY is the primary axis:
/// every row says whether it can be removed and why not, because 52 of
/// 295 worktrees here hold commits that exist nowhere else.
/// A shimmering placeholder sized to the text it stands in for.
///
/// Deliberately not a spinner per row: 289 spinners is a disco, and a
/// spinner says "something is happening" where a skeleton says "a value
/// belongs here and is coming". Respects prefers-reduced-motion via the
/// motion-safe: prefix -- an animation on every row is exactly what that
/// setting exists to stop.
function Skeleton({ className = "" }: { className?: string }) {
  return (
    <span
      aria-hidden="true"
      className={`inline-block h-3 rounded bg-[#30363d] align-middle motion-safe:animate-pulse ${className}`}
    />
  );
}

/// Why a size cell says "not measured" (#769).
///
/// One constant for both cells -- the per-repository row and the
/// all-repositories rollup -- so the explanation cannot drift between
/// two views of the same fact. It names the CAUSE rather than just the
/// outcome, because "could not measure" alone leaves the user with
/// nothing to do about it, and nesting is the cause they can act on.
const UNMEASURED_HINT =
  "This worktree could not be measured within the time limit. It is usually a very large tree, or one nested under another checkout so its files are walked twice.";

function Row({
  wt,
  repoPath,
  pr,
  onRemove,
  onClaudify,
  onForget,
  sizePending,
  sizeUnmeasurable = false,
  removing = false,
  assessed = false,
  onForce,
  onUnlock,
  onRemoveOrphan,
  onPull,
  pulling = false,
}: {
  wt: Worktree;
  /// The open pull request for this worktree, when there is one.
  /// DISPLAY ONLY -- it never feeds a safety gate.
  pr?: PullRequest | null;
  onRemove: (wt: Worktree) => void;
  onClaudify: (wt: Worktree) => void;
  onForget: (wt: Worktree) => void;
  /// This row's removal is in flight. Per row, not per page: with 100+
  /// rows, freezing all of them because one is deleting would be worse
  /// than no feedback at all.
  removing?: boolean;
  /// This worktree has been handed to Claude Code and the branch has not
  /// moved since. Unlocks the override.
  assessed?: boolean;
  onForce: (wt: Worktree) => void;
  /// Clear this worktree's lock (#775). Removes nothing; it opens a
  /// confirmation naming the holder and the age.
  onUnlock: (wt: Worktree) => void;
  /// Delete an orphaned directory. A different call from `onRemove`:
  /// git cannot remove a worktree whose repository is gone.
  onRemoveOrphan: (wt: Worktree) => void;
  /// Sizes arrive in their own pass, after safety. Tracked separately so
  /// a row whose safety has resolved does not keep waiting on its size.
  sizePending?: boolean;
  /// The walk for THIS worktree gave up before it finished (#769).
  ///
  /// Distinct from `sizePending`, and it has to be: both would otherwise
  /// render as `size_bytes === null`, and the row would go on promising
  /// a number that is never coming. That promise outlasting 15 minutes
  /// is the bug. Distinct from a measured zero too -- "could not
  /// measure" must never read as "this tree is empty", which is an
  /// invitation to delete a checkout nobody has measured.
  sizeUnmeasurable?: boolean;
  /// The repo this worktree belongs to. Needed to assess it: git has to
  /// be run from the repo, not the worktree.
  repoPath: string;
  /// Fast-forward this checkout. Only ever called for the main one.
  onPull: (wt: Worktree) => void;
  /// This row's pull is in flight. Per row, like `removing`.
  pulling?: boolean;
}) {
  const safe = isSafe(wt.safety);
  const orphaned = isOrphaned(wt.safety);
  // `Safety` already answers "is this dirty, and by how much" -- reusing
  // it means the button's reason cannot disagree with the row's own
  // explanation of the same checkout.
  const dirtyCount = wt.safety.kind === "dirty" ? wt.safety.detail : null;
  const pending = isPending(wt.safety);
  const claudifiable = canClaudify(wt.safety);
  const signal = worktreeSignal(pr);
  // Fetched only once a row is opened: several git calls each, and
  // there can be hundreds of rows on screen.
  const [open, setOpen] = useState(false);
  const { data: assessment, isLoading: assessing } = useAssessment(
    open ? repoPath : null,
    open ? wt.path : null,
    open ? wt.branch : null,
  );
  // The cells, built once. The desktop lays them out on one line; the
  // phone stacks them -- name and size, then the verdict, then the
  // action -- so the verdict can wrap instead of truncating and the
  // action is never squeezed against the path.
  const isMobile = useIsMobile();
  /// A FLOOR under the name, so it cannot be squeezed to nothing
  /// (#818).
  ///
  /// Constraining the verdict cell is most of the fix, but `flex-1` on
  /// this cell is `flex-basis: 0%` -- it is still the first thing a
  /// flex row takes width from, and two long cells competing would
  /// leave the name a few characters wide. The identifier is the one
  /// cell that is useless partially: "agent-a1..." does not distinguish
  /// the row from the nine others beside it, which is the whole failure
  /// the issue reported. 12rem is about 20 monospace characters -- long
  /// enough for the agent-worktree names this page is full of to stay
  /// told apart, and it only binds on a genuinely narrow desktop window
  /// (the phone stacks instead).
  ///
  /// A `min-width` rather than a basis, because shrink applies AFTER
  /// the basis and would walk straight back through it -- `min-width`
  /// is the only one of the two that is a floor. It does not defeat the
  /// `truncate` beside it: truncation needs the box to be allowed
  /// narrower than its TEXT, which a floor well under the text's width
  /// still allows. The phone keeps `min-w-0`, since a stacked name has
  /// the whole row and no competitor to be floored against.
  const nameCell = (
      <span
        className={`${isMobile ? "min-w-0" : "min-w-48"} flex-1 truncate font-mono text-[#e6edf3]`}
      >
        {pathBasename(wt.path)}
        {wt.branch ? (
          <span className="ml-2 text-xs text-[#8b949e]">{wt.branch}</span>
        ) : (
          <span className="ml-2 text-xs text-[#8b949e]">detached</span>
        )}
      </span>
  );
  /// The verdict as ONE plain string, for the desktop cell's tooltip.
  ///
  /// The cell below renders the same facts as composite JSX -- a PR
  /// link, a coloured signal, a relative time -- and `title` takes only
  /// text, so the pieces are re-joined here rather than read back off
  /// the DOM. Built from the same expressions the cell uses, in the
  /// same order, because a tooltip that disagrees with the row it
  /// explains is worse than no tooltip: this is the ONLY place the
  /// truncated tail survives (#818), so it has to be complete.
  const safetyTitle = [
    pending ? null : safetyReason(wt.safety),
    wt.merged_at ? `merged ${wt.merged_at}` : null,
    wt.upstream && wt.is_main ? upstreamReason(wt.upstream) : null,
    wt.upstream && !wt.is_main ? upstreamShort(wt.upstream) : null,
    pr ? `#${pr.number}` : null,
    signal ? signal.label : null,
    wt.last_commit && !wt.is_main ? relativeTime(wt.last_commit) : null,
  ]
    .filter(Boolean)
    .join(" · ");
  const safetyCell = (
      <span
        // TRUNCATES, and yields before the name does (#818).
        //
        // This cell used to be `shrink-0` with no width bound, which on
        // a flex row means "claim my full intrinsic width and give none
        // of it back". The name was the only `flex-1` cell, so it
        // absorbed all the pressure: a lock reason carrying a pid and a
        // start time squeezed the worktree's own name to nothing and
        // pushed the Remove button and kebab clean out of the bordered
        // box. You could neither tell which checkout the row was nor
        // reach its actions -- on the one page where the action deletes
        // a directory.
        //
        // So the reason now shrinks (`min-w-0`) and clips (`truncate`),
        // and the name keeps its `flex-1`. The priority is deliberate,
        // not incidental: of the three desktop cells the verdict is the
        // most expendable at narrow widths, because it is a SENTENCE --
        // its first clause carries the decision ("merged, pushed", "3
        // uncommitted files") and the tail is elaboration -- whereas the
        // name is an identifier that means nothing partially and the
        // action cell is a button that has to be clickable. A clipped
        // sentence still reads; half a name does not.
        //
        // `flex-auto` rather than `flex-1` for the basis: `flex-1` is
        // `flex: 1 1 0%`, which would give the verdict an equal share of
        // the row with the name no matter how short either one is, so a
        // two-word verdict beside a long name would sit in a half-row of
        // whitespace. `flex-auto` is `flex: 1 1 auto` -- sized from its
        // content, as it effectively was while `shrink-0`, and the
        // change is only that it can now give width back under
        // pressure. The wide row therefore looks exactly as it did
        // before this fix; only the tight one differs.
        //
        // Nothing is LOST: `title` carries the whole string. That is
        // what makes truncating honest here rather than the app hiding
        // a fact it computed -- and the mouse-free route to it is the
        // row's own disclosure, which shows the assessment in full.
        //
        // The phone is untouched. It already stacks the cells so the
        // verdict wraps instead of truncating, which is why this bug
        // was desktop-only; see the comment on the cells above.
        className={`${isMobile ? "" : "min-w-0 flex-auto truncate "}text-xs ${safetyTone(wt.safety)}`}
        // Only on the desktop, and only because that is the layout that
        // clips. Adding it on the phone would put a tooltip on text
        // already shown in full, and on a touch screen a `title` is
        // reachable by nothing.
        title={isMobile ? undefined : safetyTitle}
        // The whole row is one live region while it fills in, so a
        // screen reader hears the resolved value once rather than
        // announcing each cell as it lands.
        //
        // `aria-live` is what makes that true. `aria-busy` alone is
        // INERT on a non-live element -- it suppresses announcements
        // from a live region, so without the pairing the comment above
        // described an intent the markup never carried out.
        aria-live="polite"
        aria-busy={pending || sizePending ? true : undefined}
      >
        {pending ? <Skeleton className="w-40" /> : safetyReason(wt.safety)}
        {wt.merged_at ? (
          <span className="text-[#8b949e]"> · merged {wt.merged_at}</span>
        ) : null}
        {/* The main checkout's row said only what it was, while every
            other row earned its space. "Behind by 40" is also what
            explains why the worktrees below it are stale. */}
        {/* The main checkout gets the long prose -- it is the only thing
            that line says. Every other row gets the compact arrow form,
            since it already carries name, branch, safety, and size. */}
        {wt.upstream && wt.is_main ? (
          <span className={upstreamTone(wt.upstream)}>
            {" · "}
            {upstreamReason(wt.upstream)}
          </span>
        ) : null}
        {wt.upstream && !wt.is_main && upstreamShort(wt.upstream) ? (
          <span className={upstreamTone(wt.upstream)}>
            {" · "}
            {upstreamShort(wt.upstream)}
          </span>
        ) : null}
        {/* How stale the work is -- distinct from merged_at, which says
            whether it is already accounted for. */}
        {/* The app already holds GitHub's answer for this branch and
            never showed it here. Display only: a wrong pairing must not
            be able to authorise a deletion. */}
        {pr ? (
          <ExternalLink
            href={pr.url}
            className="text-[#58a6ff] hover:underline"
          >
            {" · "}#{pr.number}
          </ExternalLink>
        ) : null}
        {/* Why you would come back to this checkout. Display only, like
            the number itself: a wrong pairing must never authorise a
            deletion. Costs no new call -- every field read here is
            already on the pull request resolved above. */}
        {signal ? (
          <span className={signal.className}> · {signal.label}</span>
        ) : null}
        {wt.last_commit && !wt.is_main ? (
          <span className="text-[#8b949e]"> · {relativeTime(wt.last_commit)}</span>
        ) : null}
      </span>
  );
  const sizeCell = (
      <span className="w-20 shrink-0 text-right tabular-nums text-xs text-[#8b949e]">
        {/* Three states, not two. An em dash read as "measured, and the
            answer is nothing"; a skeleton says a number is still coming.
            #769 needed a third: the walk ran out of budget, so no number
            is coming and the row must stop implying one. Checked FIRST,
            because the query can still be fetching other worktrees while
            this one has already given up -- ordering it after
            `sizePending` would leave the skeleton up for the rest of the
            pass. */}
        {sizeUnmeasurable ? (
          <span className="cursor-help text-[#6e7681]" title={UNMEASURED_HINT}>
            not measured
          </span>
        ) : sizePending && wt.size_bytes === null ? (
          <Skeleton className="w-12" />
        ) : (
          formatSize(wt.size_bytes)
        )}
      </span>
  );
  const actionCells = (
    <>
      {/* One action per row, never two: the row is already dense. Safe
          rows get Remove; the 124 that cannot be removed get Claudify,
          which answers the question that actually applies to them --
          "is there anything in here worth keeping?" -- rather than
          showing a dead button that says the app will not help. */}
      {/* A FIXED-WIDTH action cell. The button in it changes label --
          "Claudify" becomes the wider "Remove anyway…" once assessed --
          and without a reserved width that swap re-flowed every column
          in the table. A row's layout should not depend on which action
          it currently offers. */}
      {/* Fixed width still, but wider: every row now holds a button AND
          a kebab, and the point of the fixed cell is that a row's
          layout never depends on which action it offers. */}
      <span
        className={
          isMobile
            ? "flex shrink-0 items-center gap-1"
            : "flex w-40 shrink-0 items-center justify-end gap-1"
        }
      >
      {claudifiable && assessed ? (
        // Only after an assessment of THIS worktree. Otherwise this is a
        // "delete anything" button with extra steps.
        //
        // The kebab beside it is the way BACK. Marking an assessment
        // used to be a one-way door: it replaced Claudify, persisted
        // across restarts, and cleared only when the branch moved -- so
        // one exploratory click permanently removed the only route to
        // that worktree's prompt. Needing the prompt again is normal.
        <button
          type="button"
          onClick={() => onForce(wt)}
          title="You assessed this worktree — remove it despite the safety gate"
          className="shrink-0 rounded border border-[#f85149]/40 px-2 py-0.5 text-xs text-[#f85149] hover:bg-[#f85149]/10"
        >
          Remove anyway…
        </button>
      ) : claudifiable ? (
        <button
          type="button"
          onClick={() => onClaudify(wt)}
          title={`Copy a prompt asking Claude Code to assess this worktree (${safetyReason(
            wt.safety,
          )})`}
          className="flex shrink-0 items-center gap-1 rounded border border-[#8957e5]/40 px-2 py-0.5 text-xs text-[#a371f7] hover:bg-[#8957e5]/10"
        >
          <Sparkles className="h-3 w-3" aria-hidden="true" />
          Claudify
        </button>
      ) : (
        <button
          type="button"
          // An ORPHAN is removable too, by a different route: git
          // cannot remove it (its repository is gone), so the Rust side
          // deletes the directory after re-checking that it is still
          // orphaned. Gating it on `safe` left the user looking at 2.5
          // GB they were told about and could not act on.
          disabled={(!safe && !orphaned) || removing}
          onClick={() => (orphaned ? onRemoveOrphan(wt) : onRemove(wt))}
          title={
            orphaned
              ? "Delete this directory — its repository is gone, so nothing about the contents can be checked first"
              : safe
                ? "Remove this worktree"
                : // A prunable row says where its action IS, which is
                  // the dead end #793 reported: the button is correctly
                  // disabled -- there is no directory to remove -- and
                  // the tooltip restating "directory is gone — prunable"
                  // told the user a diagnosis they had already read on
                  // the row, with no remedy. Now it names the header
                  // affordance, and says why the action is not here: the
                  // verb is repository-wide.
                  wt.safety.kind === "prunable"
                  ? `${safetyReason(wt.safety)} — there is nothing to remove. Use “Prune stale registrations” above the list; git prunes a whole repository at once.`
                  : safetyReason(wt.safety)
          }
          className={`shrink-0 rounded border px-2 py-0.5 text-xs ${
            (safe || orphaned) && !removing
              ? "border-[#f85149]/40 text-[#f85149] hover:bg-[#f85149]/10"
              : "border-[#30363d] text-[#8b949e] opacity-50"
          }`}
        >
          {removing ? "Removing…" : orphaned ? "Delete" : "Remove"}
        </button>
      )}
      {/* On EVERY row now, not only the assessed ones (#770).

          It used to appear solely beside "Remove anyway…", which meant
          the kebab existed exactly where removal was already on screen
          and was absent from the 124 rows where it was the missing
          affordance. Removal past the gate was reachable only from the
          Claudify toast's "I read the assessment" button — a transient
          surface for an unrecoverable action, so a stray click cost
          another agent invocation to get back.

          The ORPHANED rows keep their own Delete button as the only
          route: git cannot act on them at all, so the menu's removal
          items would be the wrong call. The kebab itself is still
          rendered there, because the Claudify pair may apply. */}
      <WorktreeKebab
        worktree={wt}
        assessed={assessed}
        onClaudify={onClaudify}
        onForget={onForget}
        onRemove={onRemove}
        onForce={onForce}
        onUnlock={onUnlock}
      />
      </span>
      {/* The disclosure, not a second action: the row keeps its
          one-action rule and this only reveals what the app already
          knows. */}
      {/* The main checkout reports how far behind it is and, until now,
          offered no way to act on it -- so fixing it meant leaving the
          app for a terminal, which is the thing this view exists to
          avoid. Its staleness is also what makes the worktrees below it
          stale.

          Disabled rather than hidden when the tree is dirty, with the
          reason in the title: an absent button just looks broken, while
          a greyed one that says "3 uncommitted files" teaches. Same
          rule `PrActions` applies to an unavailable merge.

          In practice this branch is unreachable for the button, which
          only renders when `is_main`: `classify` returns
          `Safety::MainCheckout` for the main checkout BEFORE it looks at
          `git status`, so `safety.kind` is never "dirty" there. The
          refusal the user actually meets comes from `pull_checkout`,
          which re-checks on the spot. Kept because the cost is a title
          attribute and the alternative -- deleting it and later giving
          the main checkout a real dirty verdict -- silently loses the
          explanation. */}
      {wt.is_main ? (
        <button
          type="button"
          disabled={dirtyCount !== null || pulling}
          onClick={() => onPull(wt)}
          title={
            dirtyCount !== null
              ? `${dirtyCount} uncommitted file${dirtyCount === 1 ? "" : "s"} — commit or stash first`
              : "Fast-forward this checkout to its upstream"
          }
          className={`shrink-0 rounded border px-2 py-0.5 text-xs ${
            dirtyCount !== null || pulling
              ? "border-[#30363d] text-[#8b949e] opacity-50"
              : "border-[#30363d] text-[#e6edf3] hover:bg-[#161b22]"
          }`}
        >
          {pulling ? "Updating…" : "Update to latest"}
        </button>
      ) : null}
      {/* Beside the action, since its two limits -- refuses on a dirty
          tree, never merges -- are not guessable from the label. */}
      {wt.is_main ? (
        <HelpButton topic="update-checkout" />
      ) : null}
      {claudifiable ? (
        <button
          type="button"
          onClick={() => setOpen((o) => !o)}
          aria-expanded={open}
          aria-label={`What is in ${pathBasename(wt.path)}`}
          className="shrink-0 rounded border border-[#30363d] px-2 py-0.5 text-xs text-[#8b949e] hover:bg-[#161b22]"
        >
          {open ? "Hide" : "What's in it?"}
        </button>
      ) : null}
    </>
  );
  return (
    <div className="border-b border-[#30363d] last:border-b-0">
    {isMobile ? (
      <div className="flex flex-col gap-1 px-4 py-2.5 text-sm">
        <div className="flex items-baseline gap-3">
          {nameCell}
          {sizeCell}
        </div>
        {safetyCell}
        <div className="flex flex-wrap items-center gap-2">{actionCells}</div>
      </div>
    ) : (
    <div className="flex items-baseline gap-3 px-4 py-2.5 text-sm">
      {nameCell}
      {safetyCell}
      {sizeCell}
      {actionCells}
    </div>
    )}
    {open ? (
      <div className="px-4 pb-2.5 text-xs text-[#8b949e]">
        {assessing ? (
          "Reading…"
        ) : assessment ? (
          <>
            {/* An empty summary means git answered none of it, which is
                worth saying rather than rendering a blank line. */}
            <p>{assessmentSummary(assessment) || "Nothing could be measured here."}</p>
            {assessment.subjects.length > 0 ? (
              <ul className="mt-1 list-inside list-disc">
                {assessment.subjects.map((subject, i) => (
                  <li key={`${i}-${subject}`} className="truncate">
                    {subject}
                  </li>
                ))}
                {assessment.subjects_elided > 0 ? (
                  <li className="list-none">and {assessment.subjects_elided} more</li>
                ) : null}
              </ul>
            ) : null}
          </>
        ) : (
          "Could not read this worktree."
        )}
      </div>
    ) : null}
    </div>
  );
}

/// Confirmation, naming the path and the branch.
///
/// A MODAL, not an inline banner: with 149 worktrees on one repository
/// the user clicks a row far down the page, and a prompt rendered at the
/// top is off-screen -- indistinguishable from nothing happening.
///
/// A count is not enough to act on safely either: the user needs to see
/// WHICH directory is about to disappear.
function ConfirmRemove({
  wt,
  onConfirm,
  onCancel,
}: {
  wt: Worktree;
  onConfirm: () => void;
  onCancel: () => void;
}) {
  return (
    <Dialog open onOpenChange={(open) => !open && onCancel()}>
      <DialogContent className="max-w-lg">
        <DialogTitle>Remove this worktree?</DialogTitle>
        <ActingOnDesktop />
        <p className="mt-3 break-all font-mono text-xs text-[#8b949e]">{wt.path}</p>
        <p className="mt-2 text-sm text-[#8b949e]">
          Branch <span className="font-mono">{wt.branch || "detached"}</span> is merged and
          pushed
          {wt.merged_at ? <> (merged {wt.merged_at})</> : null}, so nothing is lost.
        </p>
        <div className="mt-5 flex justify-end gap-2">
          <button
            type="button"
            onClick={onCancel}
            className="rounded border border-[#30363d] px-3 py-1.5 text-sm hover:bg-[#21262d]"
          >
            Cancel
          </button>
          <button
            type="button"
            onClick={onConfirm}
            className="rounded bg-[#da3633] px-3 py-1.5 text-sm font-medium text-white hover:bg-[#c93c37]"
          >
            Remove
          </button>
        </div>
      </DialogContent>
    </Dialog>
  );
}

export function WorktreesPage() {
  const { data: repos, isLoading, isError, error, refetch } = useWorktrees();
  const filters = useActiveFilters();
  const { setFilter } = useFilters();
  const isMobile = useIsMobile();

  const selected = repos?.find((r) => r.path === filters.repo) ?? repos?.[0];
  const {
    data: classified,
    isLoading: classifying,
    isError: classifyFailed,
    error: classifyError,
    refetch: retryClassify,
  } = useWorktreeSafety(selected?.path);
  const sizesQuery = useWorktreeSizes(selected?.path);
  // The settled answer when there is one, and the sizes streamed so far
  // when there is not. Before #754 this was `data` alone, so every row
  // held a skeleton until the LARGEST tree in the repository had been
  // walked -- MEASURED at 21.40s for one 200 GB checkout, and minutes
  // across a repository with 100 worktrees. A row's own size is known
  // long before that and there was no reason to withhold it.
  const sizes = sizesQuery.data ?? sizesQuery.partial;
  // `isFetching`, NOT `isLoading`. A DISABLED query reports `isLoading:
  // true` forever in TanStack v5 -- it has no data and never will --
  // so on "All repositories", where no repo is selected and the sizing
  // query never runs, every row waited on a request that was never
  // made. `isFetching` is true only while something is actually in
  // flight.
  const sizing = sizesQuery.isFetching;
  // The sizing pass failed outright, so NO size is coming for any row in
  // this repository. Until #769 this was never read: `useWorktreeSizes`
  // exposed `isError` and the page ignored it, so a rejected walk showed
  // skeletons while it retried and then collapsed to an em dash -- the
  // one reading the size cell's own comment says is wrong, because it
  // claims a measurement that never happened.
  const sizingFailed = sizesQuery.isError;
  const remove = useRemoveWorktree();

  /// Copy rather than spawn. The command lands in the user's own shell,
  /// where their config applies and `claude` resolves -- and there is no
  /// portable way to open "the user's terminal" anyway.
  const markRead = useMarkAssessed();
  const clearRead = useClearAssessed();

  /// Restore a worktree's Claudify action.
  ///
  /// Re-LOCKS force removal rather than unlocking it, so it is the safe
  /// direction and needs no confirmation of its own.
  const forget = (wt: Worktree) => {
    void clearRead(wt.path).then(
      () => toast.success(`${pathBasename(wt.path)} is no longer marked as assessed`),
      (e: unknown) =>
        toast.error("Could not clear the assessment", {
          description: typeof e === "string" ? e : undefined,
        }),
    );
  };

  /// "I read the assessment", the ONLY route to "Remove anyway…".
  ///
  /// Hoisted out of the clipboard's success branch, where it used to
  /// live. A failed copy took an early return, so the action vanished
  /// with it -- and the copy fails on a phone, where `copyText` reports
  /// "This window has no clipboard access." in a non-secure webview
  /// context, and on any desktop whose window is not focused. Losing
  /// the ONLY path to force-removal because the clipboard was busy is
  /// not a reasonable failure mode.
  const assessmentAction = (wt: Worktree) => ({
    label: "I read the assessment",
    onClick: () => {
      void markRead(wt.path).then(
        () => toast.success(`${pathBasename(wt.path)} can now be removed`),
        (e: unknown) =>
          toast.error("Could not record the assessment", {
            description: typeof e === "string" ? e : undefined,
          }),
      );
    },
  });

  const claudify = (wt: Worktree) => {
    claudifyCommand(selected?.path ?? "", wt.path, wt.branch).then(
      async ({ command, claude_installed }) => {
        // The phone has no terminal to paste into, and no usable
        // clipboard either. Show the command instead, framed for the
        // machine it actually runs on -- the desktop this phone drives.
        if (IS_MOBILE_BUILD) {
          setClaudifying({ worktree: wt, command, claudeInstalled: claude_installed });
          return;
        }
        // `copyText` rather than `navigator.clipboard` directly: an
        // ABSENT clipboard throws synchronously on property access, so
        // the old `.then(ok, err)` attached neither handler and the
        // click produced no toast of either kind.
        const failure = await copyText(command);
        if (failure !== null) {
          // The action rides along even here: the command could not be
          // copied, but the user can still read the assessment by
          // other means, and this is their only way to say so.
          toast.error("Could not copy the command", {
            description: failure,
            action: assessmentAction(wt),
          });
          return;
        }
        toast.success("Command copied to the clipboard", {
          // The user has to switch apps; this is the only place to say
          // so. And if Claude Code is missing, better to learn it here
          // than as a `command not found` after pasting.
          description: claude_installed
            ? "Paste it in your terminal to start the assessment."
            : "Paste it in your terminal. Claude Code was not found on this machine.",
          // The ONLY way to reach "Remove anyway…", and it is here
          // rather than automatic because that button removes a worktree
          // past its safety gate. Copying a prompt is not evidence
          // anyone read the answer; clicking this is.
          action: assessmentAction(wt),
        });
      },
      (e: unknown) =>
        toast.error("Could not build the command", {
          description: typeof e === "string" ? e : undefined,
        }),
    );
  };
  /// The command to show on the phone, which has no terminal to paste
  /// into. Null on the desktop, always: that path copies instead.
  const [claudifying, setClaudifying] = useState<{
    worktree: Worktree;
    command: string;
    claudeInstalled: boolean;
  } | null>(null);
  /// The chosen ordering, and the gesture clock that fixes it (#771).
  ///
  /// Local state rather than the `filters` store: sort here is a
  /// working choice made while clearing one repository, not a saved
  /// preference to restore on the next launch, and the store's `sort`
  /// key already means something else on the PR views.
  ///
  /// "Largest first" is the default because it is the question the page
  /// exists to answer -- on a machine with 100+ worktrees per repo,
  /// "which of these is biggest" is why you opened it.
  const [sort, setSort] = useState<WorktreeSort>("size-desc");
  /// Bumped by an explicit gesture -- choosing a sort, or clicking
  /// "re-sort" -- and by nothing else. It is what freezes the order
  /// against the size stream; see the comment on `shown`.
  const [sortedAt, setSortedAt] = useState(0);
  const chooseSort = (next: WorktreeSort) => {
    setSort(next);
    setSortedAt((n) => n + 1);
  };
  const [pending, setPending] = useState<Worktree | null>(null);
  /// The path currently being removed, or null. A path rather than a
  /// boolean so only the clicked row goes busy.
  const [removing, setRemoving] = useState<string | null>(null);
  const [bulkOpen, setBulkOpen] = useState(false);
  const [bulkBusy, setBulkBusy] = useState(false);
  const removalProgress = useRemovalProgress();
  const removeMany = useRemoveWorktrees();
  const forceRemove = useRemoveWorktreeForced();
  const { data: assessedPaths } = useAssessed();
  const { data: prs = [] } = usePullRequests();
  // Only while the confirmation is open. Docker is a subprocess call,
  // and the Worktrees page has no business paying for it just to sit
  // there -- the manifest is needed at the moment of confirming, not
  // on every render of a list nobody is acting on.
  const assessed = new Set(assessedPaths ?? []);

  // Classified data replaces the unclassified listing as it arrives, so
  // the page is useful immediately and gets more informative rather than
  // blocking on ~16s of git calls. Sizes are merged in here rather than
  // by refetching the list, so the page never flickers back to
  // unclassified.
  //
  // Computed HERE, above every early return, because the sort below is a
  // hook: React requires the same hooks in the same order on every
  // render, and the orphan and all-repositories branches return before
  // this point.
  // `has` rather than `??`: an explicit null VALUE is "the walk was
  // abandoned, no number is coming" (#769), where an absent KEY is "not
  // measured yet". `??` collapses the two, so a row that gave up would
  // fall back to its stale size and keep its skeleton forever.
  const withSizes = (classified ?? selected?.worktrees ?? []).map((w) => ({
    ...w,
    size_bytes: sizes?.has(w.path) ? (sizes.get(w.path) ?? null) : w.size_bytes,
    /// This row's own walk was abandoned, OR the whole repository's
    /// sizing pass failed. Either way no number is coming for it, and
    /// the row must say so instead of holding a skeleton.
    sizeUnmeasurable: (sizes?.has(w.path) && sizes.get(w.path) === null) || sizingFailed,
  }));

  // SORTING VS STREAMING (#771).
  //
  // Sizes land progressively (#754, #758), so a live size sort re-orders
  // rows under the cursor: the row you were reaching for slides away as
  // some other row's `du` returns. On this page that is not a cosmetic
  // annoyance -- the button it moves out from under you removes a
  // directory -- so a silent live re-sort is the one option that is
  // genuinely unsafe.
  //
  // The previous code answered this by refusing to sort at all until
  // every size was in, which is why the page could sit in path order for
  // two minutes with nothing to explain it.
  //
  // So: sort on the values known when the sort was CHOSEN, and re-sort
  // only on an explicit gesture. `sortedAt` is that gesture's clock --
  // picking a sort takes a fresh snapshot, and clicking "re-sort" takes
  // another. Between gestures the ORDER is frozen while each row's
  // displayed size keeps updating live, so a landing measurement is
  // never hidden; it simply does not move anything.
  //
  // What is memoised is the ORDER ALONE -- a list of paths -- and not
  // the rows themselves. Freezing the rows would freeze their contents
  // too, so a size landing after the snapshot would stay invisible
  // behind its skeleton until the next gesture. That is the opposite of
  // the intent: the number is never withheld, only its power to move
  // the row is.
  //
  // `orderKey` is the only dependency that may bump the memo, since the
  // freshly-built `withSizes` array is a new reference on every render
  // and listing it would restore exactly the live re-ordering this
  // exists to prevent. `length` rides along so a worktree appearing or
  // disappearing is not silently dropped from the list.
  const orderKey = `${sort}:${sortedAt}`;
  const snapshot = useMemo(
    () => ({
      order: sortWorktrees(withSizes, sort, assessed).map((w) => w.path),
      // Which paths already had a size when this order was fixed. Kept
      // alongside the order because it is the only way to tell later
      // that a measurement has landed SINCE -- comparing against the
      // live rows would compare them with themselves.
      measured: new Set(
        withSizes.filter((w) => w.size_bytes !== null && w.size_bytes !== undefined).map((w) => w.path),
      ),
    }),
    // eslint-disable-next-line react-hooks/exhaustive-deps
    [orderKey, withSizes.length],
  );
  const order = snapshot.order;
  // The live rows, in the frozen order. A path the snapshot has not
  // seen yet sorts to the end rather than being dropped -- a worktree
  // must never vanish from the list because it appeared between
  // gestures.
  const rank = new Map(order.map((p, i) => [p, i]));
  const shown = [...withSizes].sort(
    (a, b) => (rank.get(a.path) ?? Infinity) - (rank.get(b.path) ?? Infinity),
  );

  const [forcing, setForcing] = useState<Worktree | null>(null);
  /// The worktree whose lock the user is being asked to confirm
  /// clearing, or null (#775).
  ///
  /// Its own state rather than a flag on `forcing`: the two dialogs say
  /// opposite things -- one is about losing work irreversibly, the
  /// other about clearing a reversible guard -- and sharing a slot
  /// would put them one boolean away from showing the wrong one.
  const [unlocking, setUnlocking] = useState<Worktree | null>(null);
  const unlock = useUnlockWorktree();
  /// The bulk unlock's dialog and in-flight flag (#792).
  ///
  /// Two pieces of state rather than one nullable list, because the
  /// batch outlives its dialog: the dialog closes on confirm so the user
  /// gets their app back -- the same decision the bulk removal made for
  /// the same reason -- and the button then needs to know work is still
  /// running. The rows themselves come from `deadLocks` below, recomputed
  /// each render, so a freezing snapshot cannot go stale against them.
  const [bulkUnlockOpen, setBulkUnlockOpen] = useState(false);
  const [bulkUnlockBusy, setBulkUnlockBusy] = useState(false);
  const unlockMany = useUnlockWorktrees();
  /// Pruning's in-flight flag, and no dialog (#793).
  ///
  /// Deliberately the only cleanup on this page with no confirmation.
  /// Every registration it clears describes a directory git has already
  /// reported gone, so there is nothing to lose and nothing to review --
  /// and a dialog over an action with no recoverable loss is how users
  /// learn to click through the dialogs that do matter.
  const [pruning, setPruning] = useState(false);
  const prune = usePruneWorktrees();
  const [pullingPath, setPullingPath] = useState<string | null>(null);
  const removeOrphanFn = useRemoveOrphan();
  // Every repository's sizes, only on the all-repositories view. One
  // query per repository so results land progressively -- the full set
  // takes ~2 minutes, and blocking on it is what made this page look
  // stuck.
  const {
    sizes: allSizes,
    pending: sizesPending,
    total: sizesTotal,
  } = useAllWorktreeSizes(
    (repos ?? []).map((r) => r.path),
    !filters.repo,
  );

  /// Delete an orphaned directory, reporting the Rust side's own words.
  ///
  /// No confirmation dialog: nothing about the contents can be checked
  /// -- that is the definition of an orphan -- so a dialog could only
  /// repeat what the button's title already says. The re-check that
  /// matters happens in Rust, at the moment of deletion.
  const runRemoveOrphan = (wt: Worktree) => {
    setRemoving(wt.path);
    removeOrphanFn(wt.path).then(
      () => {
        setRemoving(null);
        toast.success(`Deleted ${pathBasename(wt.path)}`);
      },
      (e: unknown) => {
        setRemoving(null);
        toast.error(`Could not delete ${pathBasename(wt.path)}`, {
          description: typeof e === "string" ? e : undefined,
        });
      },
    );
  };
  /// Clear this repository's stale worktree registrations (#793).
  ///
  /// No confirmation: see `pruning`. What it does need is an HONEST
  /// result, and the count is it -- `git worktree prune` is silent on
  /// success, so a bare "Pruned" would be indistinguishable from a
  /// no-op on a repository somebody had already pruned in a terminal.
  /// The Rust side counts by listing before and after, so the number is
  /// what actually went rather than what we hoped would.
  ///
  /// A zero is reported as a neutral message, not a success and not an
  /// error. Nothing was wrong and nothing happened, and dressing that up
  /// either way would misdescribe it.
  const runPrune = () => {
    setPruning(true);
    prune(selected?.path ?? "").then(
      (cleared) => {
        setPruning(false);
        if (cleared === 0) {
          toast.info("Nothing to prune", {
            description:
              "Git found no stale registrations — they may already have been cleared.",
          });
        } else {
          toast.success(
            `Pruned ${cleared} stale registration${cleared === 1 ? "" : "s"}`,
            {
              // Says what did NOT happen as plainly as what did. The
              // word "prune" beside a list of worktrees invites reading
              // this as a deletion, and on a page whose other buttons
              // delete directories that confusion is worth one line.
              description: "No files were deleted — only git's records of directories already gone.",
            },
          );
        }
      },
      (e: unknown) => {
        setPruning(false);
        toast.error("Could not prune stale registrations", {
          description: typeof e === "string" ? e : undefined,
        });
      },
    );
  };

  const pull = usePullCheckout();

  /// Fast-forward the main checkout, reporting git's own words either
  /// way. A generic "could not update" would throw away the one part of
  /// the failure that tells the user what to do.
  const runPull = (wt: Worktree) => {
    setPullingPath(wt.path);
    pull(wt.path).then(
      (out) => {
        setPullingPath(null);
        // Summarised, not verbatim: a real fast-forward's stdout is the
        // whole diffstat, and on a busy repository that filled the
        // screen to say "it worked" (#652). `summarisePull` keeps the
        // distinction the old comment here was protecting -- git's
        // "Already up to date." is passed through, because replacing it
        // with "Updated" would claim a change that did not happen.
        toast.success(summarisePull(out));
      },
      (e: unknown) => {
        setPullingPath(null);
        toast.error(`Could not update ${pathBasename(wt.path)}`, {
          description: typeof e === "string" ? e : undefined,
        });
      },
    );
  };

  if (isLoading) {
    return (
      <div className="rounded-md border border-[#30363d] px-4 py-12 text-center text-sm text-[#8b949e]">
        Scanning for worktrees…
      </div>
    );
  }

  if (isError) {
    return (
      <QueryError
        title="Could not scan for worktrees"
        message={errorMessage(error)}
        onRetry={() => void refetch()}
      />
    );
  }

  if (!repos || repos.length === 0) {
    return (
      <div className="rounded-md border border-[#30363d] px-4 py-12 text-center">
        <p className="text-sm font-semibold text-[#e6edf3]">No repositories found</p>
        <p className="mx-auto mt-2 max-w-md text-sm text-[#8b949e]">
          Set the directories to scan in Settings, at the bottom right.
        </p>
      </div>
    );
  }

  // "All repositories": a genuine rollup rather than the first repo.
  //
  // Deliberately READ-ONLY. Safety classification is per repo and costs
  // ~16s across all of them, so this view has no verdicts -- and every
  // removal path in this app is gated on a verdict. Offering Remove here
  // would mean either deleting without a safety check or blocking the
  // view on a 16-second scan; showing where the disk went, and sending
  // the user into the repo to act, is neither.
  // The Orphaned section. Its own branch rather than a repo page: an
  // orphan belongs to no repository, so `selected` cannot express it,
  // and the per-repo page's safety affordances do not apply.
  if (filters.repo === ORPHAN_FILTER) {
    const orphans = (repos ?? [])
      .flatMap((r) => r.worktrees)
      .filter((w) => isOrphaned(w.safety));
    return (
      <div className="rounded-md border border-[#30363d]">
        <div className="border-b border-[#30363d] px-4 py-3">
          <span className="text-sm font-semibold text-[#e6edf3]">
            {orphans.length} orphaned worktree{orphans.length === 1 ? "" : "s"}
            {/* The most important help in the app: this is the only
                place it offers a delete having verified nothing. */}
            <HelpButton topic="orphaned-worktrees" />
          </span>
          {/* Says what these ARE before offering to delete them. The
              row's own reason says nothing can be checked; this says
              why that is, which is what makes the Delete button a
              considered choice rather than a gamble. */}
          <p className="mt-1 text-xs text-[#8b949e]">
            The repository each of these belonged to has been deleted, so git can
            no longer read them — not whether they hold uncommitted work, and not
            whether their branch ever merged. Deleting one removes the directory
            outright.
          </p>
        </div>
        {orphans.map((wt) => (
          <Row
            key={wt.path}
            wt={wt}
            repoPath=""
            assessed={false}
            onForce={setForcing}
            onUnlock={setUnlocking}
            onRemoveOrphan={runRemoveOrphan}
            onPull={runPull}
            onRemove={setPending}
            onClaudify={claudify}
              onForget={forget}
            removing={removing === wt.path}
          />
        ))}
      </div>
    );
  }

  if (!filters.repo) {
    // Sizes merged in as each repository answers. MEASURED: the full
    // set takes ~2 minutes, so awaiting it would show dashes for that
    // long with nothing to explain them -- which is what was reported.
    // `has`, not `??`. A worktree the walk gave up on (#769) arrives as
    // an explicit null VALUE, and `??` would fall straight through it to
    // the stale `w.size_bytes` -- erasing the one fact that stops the
    // row promising a number. An ABSENT key still means "not measured
    // yet" and must keep the existing value.
    const withSizes = repos.map((r) => ({
      ...r,
      worktrees: r.worktrees.map((w) => ({
        ...w,
        size_bytes: allSizes.has(w.path) ? (allSizes.get(w.path) ?? null) : w.size_bytes,
      })),
    }));
    const { worktrees, totalBytes, sizesComplete } = rollupRepos(withSizes);
    return (
      <div className="rounded-md border border-[#30363d]">
        <div className="flex items-baseline justify-between border-b border-[#30363d] px-4 py-3">
          <span className="text-sm font-semibold text-[#e6edf3]">
            {worktrees.length} worktree{worktrees.length === 1 ? "" : "s"} across{" "}
            {repos.length} repositor{repos.length === 1 ? "y" : "ies"}
          </span>
          <span className="text-xs text-[#8b949e]">
            {/* "at least" while any size is still unmeasured: a total
                that silently counts unknowns as zero is a confident
                wrong answer. */}
            {sizesComplete ? "" : "at least "}
            {formatSize(totalBytes)}
          </span>
        </div>
        {/* Sizes arrive one repository at a time, and this says how
            many are still outstanding.
            
            MEASURED: the full set takes ~2 minutes on a real machine
            (158 worktrees, `du` per worktree). Awaiting all of it
            showed dashes for that long with nothing to explain them.
            A count that visibly falls is the difference between "still
            working" and "broken". */}
        {sizesPending > 0 ? (
          <p className="border-b border-[#30363d] px-4 py-2 text-xs text-[#8b949e]">
            Measuring sizes — {sizesPending} of {sizesTotal} repositor
            {sizesTotal === 1 ? "y" : "ies"} still to go.
          </p>
        ) : null}
        {worktrees.map((wt) => (
          <button
            type="button"
            key={wt.path}
            onClick={() => setFilter("repo", wt.repoPath)}
            title="Open this repository to act on it"
            // On the phone the row wraps: repository and size on the
            // first line, the worktree's path on its own line beneath,
            // where it has the full width rather than what a fixed
            // 10rem repository column leaves over.
            className={
              isMobile
                ? "flex w-full flex-wrap items-baseline gap-x-3 gap-y-0.5 border-b border-[#30363d] px-4 py-2.5 text-left text-sm last:border-b-0 hover:bg-[#161b22]"
                : "flex w-full items-baseline gap-3 border-b border-[#30363d] px-4 py-2.5 text-left text-sm last:border-b-0 hover:bg-[#161b22]"
            }
          >
            <span
              className={
                isMobile
                  ? "min-w-0 flex-1 truncate text-[#8b949e]"
                  : "w-40 shrink-0 truncate text-[#8b949e]"
              }
            >
              {wt.repoName}
            </span>
            <span
              className={
                isMobile
                  ? "order-last min-w-0 basis-full truncate font-mono text-[#e6edf3]"
                  : "min-w-0 flex-1 truncate font-mono text-[#e6edf3]"
              }
            >
              {pathBasename(wt.path)}
            </span>
            <span className="w-20 shrink-0 text-right tabular-nums text-xs text-[#8b949e]">
              {/* The same three states as the per-repository cell. Once
                  the pass has finished, a null here is no longer "still
                  coming" -- it is a walk that was abandoned (#769), and
                  an em dash would read as "measured, and the answer is
                  nothing". The banner above covers the pending case, so
                  this only has to separate the other two. */}
              {wt.size_bytes === null && sizesPending === 0 ? (
                <span className="cursor-help text-[#6e7681]" title={UNMEASURED_HINT}>
                  not measured
                </span>
              ) : wt.size_bytes === null ? (
                "—"
              ) : (
                formatSize(wt.size_bytes)
              )}
            </span>
          </button>
        ))}
      </div>
    );
  }

  // How many rows have gained a size since the current order was fixed.
  //
  // This is what makes the frozen order HONEST rather than merely
  // stable: without it the user reads a "Largest first" list that is
  // quietly out of date and has no way to know. With it, the page says
  // so and offers the one click that fixes it. Only meaningful on a
  // size sort -- name never goes stale, and `last_commit` arrives with
  // the row rather than streaming in afterwards.
  const sizeSorted = sort === "size-asc" || sort === "size-desc";
  const restaleCount = sizeSorted
    ? withSizes.filter(
        (w) =>
          w.size_bytes !== null &&
          w.size_bytes !== undefined &&
          !snapshot.measured.has(w.path),
      ).length
    : 0;

  // Withheld unless classification actually SUCCEEDED. A failed pass
  // used to resolve as an empty success, so rows sat on "checking..."
  // forever while this read a confident "0 safe to remove".
  const safeKnown = !classifying && !classifyFailed;
  const shownSafe = shown.filter((w) => isSafe(w.safety));
  const safeCount = shownSafe.length;
  /// Stale registrations, counted and labelled SEPARATELY from "safe to
  /// remove" (#793).
  ///
  /// Their own count on purpose, and the issue is explicit that this is
  /// the right shape: a prunable worktree is not unsafe, it is a
  /// different verb. `is_safe()` stays a two-variant allowlist, the
  /// Remove button stays disabled on these rows because there is no
  /// directory to remove, and the thing that was missing was never a
  /// wider gate -- it was an action and a number of its own. Folding
  /// them into the green count would claim 12 directories are
  /// recoverable disk when they are 12 dangling pointers.
  const shownPrunable = shown.filter((w) => w.safety.kind === "prunable");
  const prunableCount = shownPrunable.length;
  /// Locks whose named holder is provably gone (#792).
  ///
  /// `isDeadLock` narrows to `holder_running === false` -- see its own
  /// doc for why `null` must not count. These rows are the ones a bulk
  /// unlock may touch; every other locked row keeps the single-row
  /// confirmation, because a bulk action over claims that might be live
  /// is exactly what #753 refused to offer and was right to.
  const shownDeadLocks = shown.filter(isDeadLock);
  const deadLockCount = shownDeadLocks.length;
  // Same honesty rule as the all-repositories rollup: an unmeasured
  // size is null, and counting it as zero would report a confident
  // wrong total. Sizes arrive in their own pass after safety, so this
  // grows as results land rather than being computed once.
  const measured = shown.filter((w) => w.size_bytes !== null && w.size_bytes !== undefined);
  const totalBytes = measured.reduce((n, w) => n + (w.size_bytes ?? 0), 0);
  const sizesComplete = measured.length === shown.length;

  return (
    <div className="flex flex-col gap-3">
      <div className="flex flex-wrap items-baseline gap-3 text-sm">
        <span className="font-semibold">{selected?.name}</span>
        <span className="text-[#8b949e]">
          {shown.length} worktree{shown.length === 1 ? "" : "s"}
        </span>
        {/* How old these answers are.
            
            Every verdict on the rows below is computed against refs on
            disk; the scan never fetches, on purpose. Saying nothing
            made a fortnight-old answer read like the present tense
            (#702). Silent under a day, because a caveat shown always is
            a caveat nobody reads. */}
        {refAge(selected?.fetched_at ?? null) === null ? null : (
          <span
            className="text-xs text-[#d29922]"
            title="Merge and upstream verdicts are computed from the refs on disk, which are only as current as the last fetch."
          >
            {refAge(selected?.fetched_at ?? null)}
          </span>
        )}
        {/* The count is withheld, not shown as a growing number: a
            "3 safe to remove" that climbs to 122 as rows resolve invites
            acting on a figure that was never the answer. */}
        {classifying ? (
          <span className="text-xs text-[#58a6ff]">checking what is safe to remove…</span>
        ) : classifyFailed ? (
          <button
            type="button"
            onClick={() => void retryClassify()}
            className="text-xs text-[#f85149] hover:underline"
            title={errorMessage(classifyError)}
          >
            could not check what is safe — retry
          </button>
        ) : (
          <span className="inline-flex items-center text-xs text-[#3fb950]">
            {safeCount} safe to remove
            {/* This view deletes directories on the strength of a
                one-line verdict. The rules behind it run to several
                paragraphs and lived only in source comments. */}
            <HelpButton topic="worktree-safety" />
          </span>
        )}
        {/* A SECOND count, in its own words and its own colour (#793).

            "12 stale registrations" is not a subset of "safe to remove"
            and must not read as one: there is no directory behind these,
            so they are not disk to reclaim and the Remove button is
            rightly disabled on them. Grey, matching the rows' own tone,
            because nothing here is at risk and nothing is being asked of
            the user's judgement.

            Silent at zero. Most repositories have none, and a permanent
            "0 stale registrations" would be furniture. */}
        {safeKnown && prunableCount > 0 ? (
          <span className="text-xs text-[#8b949e]">
            {prunableCount} stale registration{prunableCount === 1 ? "" : "s"}
          </span>
        ) : null}
        {/* The action #793 found missing entirely. `git worktree prune`
            appeared in three prose comments in this repository and in no
            argument list anywhere, so the app diagnosed the condition,
            named the remedy, disabled the button that would be wrong,
            and sent the user to a terminal.

            ONE affordance over the repository rather than a button per
            row, because that is the shape of git's verb: `prune` takes
            no path and clears every stale registration. A per-row button
            would have cleared all 12 and said it cleared one.

            No confirmation behind it, unlike every other cleanup on this
            page -- see `pruning`'s own note. Styled as an ordinary
            action, not a destructive one: red here would put a dangling
            pointer in the same visual class as deleting a directory
            full of unpushed commits. */}
        {safeKnown && prunableCount > 0 ? (
          <button
            type="button"
            disabled={pruning}
            onClick={() => runPrune()}
            title="Run `git worktree prune` — clears registrations whose directory is already gone. Nothing on disk is deleted."
            className="rounded border border-[#30363d] px-2 py-0.5 text-xs text-[#e6edf3] hover:bg-[#161b22] disabled:opacity-50"
          >
            {pruning
              ? "Pruning…"
              : `Prune ${prunableCount} stale registration${prunableCount === 1 ? "" : "s"}`}
          </button>
        ) : null}
        {/* Dead-holder locks, counted separately again and for the same
            reason (#792): this is not disk to reclaim, it is an obstacle
            that has stopped being one. Grey, matching these rows' new
            tone -- a lock nothing holds is bookkeeping, and amber would
            keep asking for the judgement that is no longer required. */}
        {safeKnown && deadLockCount > 0 ? (
          <span className="text-xs text-[#8b949e]">
            {deadLockCount} lock{deadLockCount === 1 ? "" : "s"} with no live holder
          </span>
        ) : null}
        {/* The low-friction route for the provably-dead case (#792).

            A BULK affordance because the condition is bulk: five on the
            reporting machine after one reboot, 20+ historically, and
            clicking through five separate confirmations that each say
            "the process it names is no longer running" adds no judgement
            — it only adds clicks to a decision already made.

            Still behind a dialog, unlike Prune, and the line between
            them is real: pruning clears a pointer to a directory that is
            gone, while this clears a claim on a directory that is still
            there. The dialog lists the paths, so the bulk action is
            reviewed once rather than not at all.

            Amber, like the single-row unlock item: reversible by `git
            worktree lock`, so not red, but not a plain action either. */}
        {safeKnown && deadLockCount > 0 ? (
          <button
            type="button"
            disabled={bulkUnlockBusy}
            onClick={() => setBulkUnlockOpen(true)}
            className="rounded border border-[#d29922]/40 px-2 py-0.5 text-xs text-[#d29922] hover:bg-[#d29922]/10 disabled:opacity-50"
          >
            {bulkUnlockBusy
              ? "Unlocking…"
              : `Unlock ${deadLockCount} abandoned lock${deadLockCount === 1 ? "" : "s"}`}
          </button>
        ) : null}
        {/* A COUNT, not a bare "measuring sizes…".

            The old label said only that work was happening, which is
            indistinguishable from a hang once it has said it for ten
            minutes -- and that is precisely what #754 reported. The walk
            is unbounded in wall-clock terms (MEASURED: 21.40s for a
            single 200 GB checkout), so the honest thing is not to
            promise a finish time but to show it advancing. A number that
            visibly falls is the difference between "still working" and
            "broken". */}
        {!classifying && sizing ? (
          <span className="text-xs text-[#8b949e]">
            measuring sizes — {shown.length - measured.length} of {shown.length} to go
          </span>
        ) : null}
        {/* "at least" until every worktree has been measured. Reported
            here as well as on the rollup, because the per-repo page is
            where you land after choosing a repo and could not answer
            "how much is this one holding?". */}
        {measured.length > 0 ? (
          <span className="text-xs text-[#8b949e]">
            {sizesComplete ? "" : "at least "}
            {formatSize(totalBytes)} total
          </span>
        ) : null}

        {/* The three columns the row already renders, both ways (#771).
            A `<select>` rather than clickable column headers: this list
            is not a table -- the safety verdict is prose that wraps, and
            there are no headers to click. `ArtifactsPage` reached the
            same shape for the same reason, so this matches it rather
            than inventing a third pattern. */}
        <label className="flex items-center gap-1 text-xs text-[#8b949e]">
          Sort
          <select
            value={sort}
            onChange={(e) => chooseSort(e.target.value as WorktreeSort)}
            aria-label="Sort worktrees"
            className="rounded border border-[#30363d] bg-[#0d1117] px-1 py-0.5 text-xs text-[#e6edf3]"
          >
            {(Object.keys(WORKTREE_SORT_LABELS) as WorktreeSort[]).map((k) => (
              <option key={k} value={k}>
                {WORKTREE_SORT_LABELS[k]}
              </option>
            ))}
          </select>
        </label>

        {/* The explicit gesture that makes the frozen order honest.

            Rows are ordered on the sizes known when the sort was
            chosen, so a measurement landing afterwards does not move
            anything under the cursor -- but it would leave a "Largest
            first" list quietly out of date with no way to tell. This
            says how many rows have been measured since, and one click
            applies them. Silent when there is nothing to apply, so it
            is not a permanent piece of furniture. */}
        {restaleCount > 0 ? (
          <button
            type="button"
            onClick={() => setSortedAt((n) => n + 1)}
            aria-live="polite"
            className="rounded border border-[#30363d] px-2 py-0.5 text-xs text-[#58a6ff] hover:bg-[#161b22]"
          >
            re-sort — {restaleCount} newly measured
          </button>
        ) : null}

        {/* The count is in the label, so the scope is legible before
            clicking rather than only in the dialog. 106 of 268 worktrees
            are safe on a real machine, mostly in a few repos -- clicking
            those one at a time adds no safety, only clicks. */}
        {safeCount > 1 && safeKnown ? (
          <button
            type="button"
            disabled={bulkBusy}
            onClick={() => setBulkOpen(true)}
            className="rounded border border-[#f85149]/40 px-2 py-0.5 text-xs text-[#f85149] hover:bg-[#f85149]/10 disabled:opacity-50"
          >
            {/* A count, not a spinner: ~100 worktrees is around 30
                seconds of sequential deletion, and a bare "Removing…"
                for that long is indistinguishable from a hang. */}
            {bulkBusy
              ? removalProgress
                ? `Removed ${removalProgress.done} of ${removalProgress.total}…`
                : "Removing…"
              : `Remove ${safeCount} safe worktree${safeCount === 1 ? "" : "s"}`}
          </button>
        ) : null}
        {/* Beside the button rather than in the dialog: the question
            ("can I leave this page?") occurs while it is running, which
            is when the dialog is already gone. */}
        {safeCount > 1 && safeKnown ? <HelpButton topic="bulk-removal" /> : null}

        <button
          type="button"
          onClick={() => setFilter("repo", undefined)}
          className="ml-auto text-xs text-[#8b949e] hover:text-[#e6edf3]"
        >
          All repositories
        </button>
      </div>

      {/* Per worktree, not bulk. Bulk-deleting directories is where a
          wrong predicate becomes unrecoverable at scale, and with 149
          removable worktrees on one repo the temptation is real. */}
      {claudifying !== null ? (
        <Dialog open onOpenChange={() => setClaudifying(null)}>
          <DialogContent className="max-w-lg">
            <DialogTitle>Assess {pathBasename(claudifying.worktree.path)}</DialogTitle>
            <ActingOnDesktop />
            <p className="text-sm text-[#8b949e]">
              {claudifying.claudeInstalled
                ? "Run this on that desktop to have Claude Code assess what removing this worktree would lose."
                : "Run this on that desktop. Claude Code was not found there, so it may need installing first."}
            </p>
            {/* Selectable, and wrapped rather than truncated: the point
                is that a person can read and retype it. No copy button
                -- `copyText` is exactly what does not work here, and
                offering one that fails is worse than offering none. */}
            <pre className="max-h-48 overflow-auto rounded border border-[#30363d] bg-[#0d1117] p-3 text-xs break-all whitespace-pre-wrap select-text">
              {claudifying.command}
            </pre>
            <div className="flex justify-end gap-2">
              <Button
                variant="outline"
                className="min-h-11"
                onClick={() => {
                  const wt = claudifying.worktree;
                  setClaudifying(null);
                  assessmentAction(wt).onClick();
                }}
              >
                I read the assessment
              </Button>
              <Button variant="ghost" className="min-h-11" onClick={() => setClaudifying(null)}>
                Close
              </Button>
            </div>
          </DialogContent>
        </Dialog>
      ) : null}
      {pending ? (
        <ConfirmRemove
          wt={pending}
          onCancel={() => setPending(null)}
          onConfirm={() => {
            const target = pending;
            const name = pathBasename(target.path);
            setPending(null);
            setRemoving(target.path);
            remove(selected?.path ?? "", target.path).then(
              () => {
                setRemoving(null);
                toast.success(`Removed ${name}`);
              },
              // The backend re-checks safety at delete time, so a
              // worktree that went dirty since the scan is refused. That
              // message is the useful part -- show it, do not summarise.
              (e: unknown) => {
                // Back to normal, not stuck on "Removing...": the backend
                // re-checks safety at delete time and legitimately
                // refuses a worktree that went dirty since the scan.
                setRemoving(null);
                toast.error(`Could not remove ${name}`, {
                  description: typeof e === "string" ? e : undefined,
                });
              },
            );
          }}
        />
      ) : null}

      {forcing ? (
        <Dialog open onOpenChange={(o) => !o && setForcing(null)}>
          <DialogContent className="max-w-lg">
            <DialogTitle>Remove {pathBasename(forcing.path)}?</DialogTitle>
            <ActingOnDesktop />
            <p className="mt-3 break-all font-mono text-xs text-[#8b949e]">{forcing.path}</p>
            {/* The specific loss, computed now -- "are you sure?" is not
                something anyone can act on, and this is the only
                genuinely unrecoverable case in the app. */}
            <p className="mt-3 text-sm text-[#e6edf3]">
              {safetyReason(forcing.safety)}
              {forcing.upstream && upstreamShort(forcing.upstream)
                ? ` · ${upstreamShort(forcing.upstream)}`
                : ""}
              {forcing.last_commit ? ` · last commit ${relativeTime(forcing.last_commit)}` : ""}
            </p>
            {/* An empty branch is the one case here where nothing is at
                risk, so it gets neither the red nor the warning. Saying
                "these commits are not pushed anywhere" over a branch
                with no commits is the exact misreport of #701, and
                repeating it in the confirmation is worse than in the
                row: this is the moment the user decides. */}
            <p
              className={`mt-2 text-sm ${
                forcing.safety.kind === "empty" ? "text-[#8b949e]" : "text-[#f85149]"
              }`}
            >
              {forceWarning(forcing.safety)}
            </p>
            <div className="mt-5 flex justify-end gap-2">
              <button
                type="button"
                onClick={() => setForcing(null)}
                className="rounded border border-[#30363d] px-3 py-1.5 text-sm hover:bg-[#21262d]"
              >
                Cancel
              </button>
              <button
                type="button"
                onClick={() => {
                  const target = forcing;
                  const name = pathBasename(target.path);
                  setForcing(null);
                  setRemoving(target.path);
                  forceRemove(selected?.path ?? "", target.path).then(
                    () => {
                      setRemoving(null);
                      toast.success(`Removed ${name}`);
                    },
                    (e: unknown) => {
                      setRemoving(null);
                      toast.error(`Could not remove ${name}`, {
                        description: typeof e === "string" ? e : undefined,
                      });
                    },
                  );
                }}
                className="rounded bg-[#da3633] px-3 py-1.5 text-sm font-medium text-white hover:bg-[#c93c37]"
              >
                I have reviewed this — remove it
              </button>
            </div>
          </DialogContent>
        </Dialog>
      ) : null}

      {/* The unlock confirmation (#775).

          #753 declined an unlock action on the grounds that a button
          beside a row invites clearing a claim without reading it. This
          dialog is the answer to that objection rather than a way
          around it: it exists to make the user LOOK at what they are
          clearing, so it leads with the holder and the age and says
          what is underneath before offering the button.

          Deliberately NOT styled as a destructive confirmation. Nothing
          is deleted, `git worktree lock` puts it back, and dressing a
          reversible action in the red of the one unrecoverable action
          on this page would teach the user to discount both. The
          caution here is a different one -- another process may be
          using that directory -- and it is stated in those words. */}
      {unlocking && unlocking.safety.kind === "locked" ? (
        <Dialog open onOpenChange={(o) => !o && setUnlocking(null)}>
          <DialogContent className="max-w-lg">
            <DialogTitle>Unlock {pathBasename(unlocking.path)}?</DialogTitle>
            <ActingOnDesktop />
            <p className="mt-3 break-all font-mono text-xs text-[#8b949e]">
              {unlocking.path}
            </p>

            {/* WHO and WHEN, as two separate lines, because they are
                two separate pieces of evidence and one of them is much
                better than the other.

                The age leads. It is the only part that differs per row
                and that a long-lived parent process cannot fake --
                measured, the locks on the reporting machine span four
                days while every one of them names the same pid. */}
            <p className="mt-3 text-sm text-[#e6edf3]">
              Locked {lockAge(unlocking.safety.detail) ?? "at an unknown time"}
              {unlocking.safety.detail.reason === null ? (
                <>, with no reason given.</>
              ) : (
                <>
                  {" "}
                  by{" "}
                  <span className="font-mono text-xs">
                    {unlocking.safety.detail.reason}
                  </span>
                </>
              )}
            </p>

            {/* What checking the pid actually established -- phrased so
                a running process cannot be mistaken for proof that the
                lock is live. On the reporting machine this reads
                "still running" for all 20 locks and every one of them
                is abandoned, which is exactly why the sentence carries
                its own caveat. */}
            {lockHolderNote(unlocking.safety.detail) ? (
              <p className="mt-2 text-sm text-[#8b949e]">
                {lockHolderNote(unlocking.safety.detail)}
              </p>
            ) : null}

            {/* What is UNDERNEATH -- the thing #753 could not tell the
                user, and the reason unlocking was a blind action. */}
            <p className="mt-2 text-sm text-[#8b949e]">
              Underneath the lock, this worktree is{" "}
              <span className="text-[#e6edf3]">
                {safetyReason(unlocking.safety.detail.underlying)}
              </span>
              .
            </p>

            {/* The honest caution. Not "this cannot be undone" -- it
                plainly can -- but the risk that is real: the lock is
                how another process says it is using this directory. */}
            <p className="mt-3 text-sm text-[#d29922]">
              Unlocking deletes nothing and can be undone by locking it
              again. It does clear the only signal another process has for
              saying it is working here — so check the holder above before
              clearing it.
            </p>

            <div className="mt-5 flex justify-end gap-2">
              <button
                type="button"
                onClick={() => setUnlocking(null)}
                className="rounded border border-[#30363d] px-3 py-1.5 text-sm hover:bg-[#21262d]"
              >
                Cancel
              </button>
              <button
                type="button"
                onClick={() => {
                  const target = unlocking;
                  const name = pathBasename(target.path);
                  setUnlocking(null);
                  unlock(selected?.path ?? "", target.path).then(
                    () =>
                      toast.success(`Unlocked ${name}`, {
                        // Says what changed and what did NOT: the row
                        // is about to re-classify, and the user should
                        // not read a successful unlock as a removal.
                        description:
                          "The worktree is still there — its safety verdict will refresh.",
                      }),
                    (e: unknown) =>
                      toast.error(`Could not unlock ${name}`, {
                        description: typeof e === "string" ? e : undefined,
                      }),
                  );
                }}
                className="rounded border border-[#d29922]/40 px-3 py-1.5 text-sm font-medium text-[#d29922] hover:bg-[#d29922]/10"
              >
                Unlock it
              </button>
            </div>
          </DialogContent>
        </Dialog>
      ) : null}

      {/* The bulk unlock confirmation (#792).

          Narrower than it looks: only rows where `holder_running` is
          explicitly false get here, so every line in the list below has
          a named process that has been checked and is not running. A
          lock whose holder is alive, or whose reason named nobody to
          check, is excluded and keeps the single-row dialog -- a bulk
          action over claims that MIGHT be live is exactly what #753
          refused to offer, and nothing here reopens that.

          So the dialog's job is narrower than the single-row one's. That
          one exists to make the user read evidence and weigh it; this
          one states the evidence is already decisive and shows the
          scope, because the remaining question is "which directories"
          rather than "should I". Hence the path list and no per-row
          holder prose.

          Deliberately NOT styled as destructive, for the reason the
          single-row dialog gives: nothing is deleted and `git worktree
          lock` puts it back. */}
      {bulkUnlockOpen ? (
        <Dialog open onOpenChange={(o) => !o && setBulkUnlockOpen(false)}>
          <DialogContent className="max-w-2xl">
            <DialogTitle>
              Unlock {deadLockCount} abandoned lock{deadLockCount === 1 ? "" : "s"}?
            </DialogTitle>
            <ActingOnDesktop />
            <p className="mt-2 text-sm text-[#8b949e]">
              Each of these locks names a process, and each of those processes is
              no longer running — so nothing is working in these directories.
              Unlocking deletes nothing and can be undone by locking them again.
            </p>
            {/* What the unlock does NOT do, said before the button
                rather than only in the toast afterwards. On a page where
                the adjacent bulk button deletes directories, a user
                clicking this one deserves to know the rows will still be
                there. */}
            <p className="mt-2 text-sm text-[#8b949e]">
              The worktrees stay where they are. Their safety verdicts are
              re-checked afterwards, and each is removable only if it earns that
              on its own.
            </p>
            {/* Every path. These are claims on real directories, and a
                count alone would make the scope unreviewable. */}
            <ul className="mt-3 max-h-64 overflow-y-auto font-mono text-xs text-[#8b949e]">
              {shownDeadLocks.map((w) => (
                <li key={w.path} className="py-0.5">
                  {w.path}
                </li>
              ))}
            </ul>
            <div className="mt-5 flex justify-end gap-2">
              <button
                type="button"
                onClick={() => setBulkUnlockOpen(false)}
                className="rounded border border-[#30363d] px-3 py-1.5 text-sm hover:bg-[#21262d]"
              >
                Cancel
              </button>
              <button
                type="button"
                onClick={() => {
                  const targets = shownDeadLocks.map((w) => w.path);
                  // Closed NOW, like the bulk removal and for the same
                  // reason: the work does not need the dialog, progress
                  // is on the toolbar button, and twenty sequential
                  // unlocks should not hold the app.
                  setBulkUnlockBusy(true);
                  setBulkUnlockOpen(false);
                  unlockMany(selected?.path ?? "", targets).then(
                    (outcomes) => {
                      setBulkUnlockBusy(false);
                      const failed = outcomes.filter((o) => o.error !== null);
                      const ok = outcomes.length - failed.length;
                      // Never a bare "done". A row somebody else
                      // unlocked between the scan and the click is an
                      // ordinary race, and hiding the refusals would
                      // misreport which locks are still in place.
                      if (failed.length === 0) {
                        toast.success(`Unlocked ${ok} worktree${ok === 1 ? "" : "s"}`, {
                          description:
                            "The worktrees are still there — their safety verdicts will refresh.",
                        });
                      } else {
                        toast.error(
                          `${failed.length} of ${outcomes.length} could not be unlocked`,
                          {
                            description: failed
                              .map((f) => `${pathBasename(f.path)}: ${f.error}`)
                              .join("\n"),
                          },
                        );
                      }
                    },
                    (e: unknown) => {
                      setBulkUnlockBusy(false);
                      if (isCancelled(e)) return;
                      toast.error("The bulk unlock could not run", {
                        description: typeof e === "string" ? e : undefined,
                      });
                    },
                  );
                }}
                disabled={bulkUnlockBusy}
                className="rounded border border-[#d29922]/40 px-3 py-1.5 text-sm font-medium text-[#d29922] hover:bg-[#d29922]/10 disabled:opacity-50"
              >
                {bulkUnlockBusy
                  ? "Unlocking…"
                  : `Unlock ${deadLockCount} lock${deadLockCount === 1 ? "" : "s"}`}
              </button>
            </div>
          </DialogContent>
        </Dialog>
      ) : null}

      {bulkOpen ? (
        <Dialog open onOpenChange={(o) => !o && setBulkOpen(false)}>
          <DialogContent className="max-w-2xl">
            <DialogTitle>
              Remove {safeCount} safe worktree{safeCount === 1 ? "" : "s"}?
            </DialogTitle>
            <ActingOnDesktop />
            <p className="mt-2 text-sm text-[#8b949e]">
              {/* Sizes are already computed, and reclaimed space is the
                  number that makes this decision -- it is why the view
                  exists. */}
              {/* "Still measuring" and "nothing to reclaim" are
                  different answers and used to share one dash. */}
              {(() => {
                const total = totalSize(shown.filter((w) => isSafe(w.safety)));
                return total === null
                  ? "Sizes are still being measured."
                  : `Reclaims ${formatSize(total)}.`;
              })()}{" "}
              Each is re-checked before deletion, so anything that changed
              since the scan is skipped.
            </p>
            {/* The third system this page could never reach. Docker
                images built from these worktrees outlive them, and
                until now removing them meant going to another view and
                working out by hand which ones belonged to what.

                Named, not just counted, and listed separately from the
                paths: this is a SECOND irreversible action in a dialog
                that used to perform one, and folding it in silently
                would be exactly the unreviewed bulk delete the manifest
                exists to prevent. */}
            {/* Every path, not a count: these are directories on disk. */}
            <ul className="mt-3 max-h-64 overflow-y-auto font-mono text-xs text-[#8b949e]">
              {shownSafe.map((w) => (
                <li key={w.path} className="py-0.5">
                  {w.path}
                </li>
              ))}
            </ul>
            <div className="mt-5 flex justify-end gap-2">
              <button
                type="button"
                onClick={() => setBulkOpen(false)}
                className="rounded border border-[#30363d] px-3 py-1.5 text-sm hover:bg-[#21262d]"
              >
                Cancel
              </button>
              <button
                type="button"
                onClick={() => {
                  const targets = shownSafe.map((w) => w.path);
                  // Closed NOW, not when the batch finishes.
                  //
                  // It used to close in `.then()`, so the modal sat over
                  // the app for the whole removal -- around 30 seconds
                  // for ~100 worktrees -- and nothing else could be
                  // used. The work does not need the dialog: progress
                  // is already on the toolbar button ("Removed 12 of
                  // 40…"), the outcomes arrive as a toast, and the
                  // promise runs to completion regardless of what is on
                  // screen. So the user asked for a deletion and gets
                  // their app back.
                  setBulkBusy(true);
                  setBulkOpen(false);
                  removeMany(selected?.path ?? "", targets).then(
                    (outcomes) => {
                      setBulkBusy(false);
                      const failed = outcomes.filter((o) => o.error !== null);
                      const ok = outcomes.length - failed.length;
                      // Never a bare "done": a worktree that went dirty
                      // since the scan is refused, and hiding that would
                      // misreport what is still on disk.
                      if (failed.length === 0) {
                        toast.success(`Removed ${ok} worktree${ok === 1 ? "" : "s"}`);
                      } else {
                        toast.error(`${failed.length} of ${outcomes.length} could not be removed`, {
                          description: failed
                            .map((f) => `${pathBasename(f.path)}: ${f.error}`)
                            .join("\n"),
                        });
                      }
                    },
                    (e: unknown) => {
                      setBulkBusy(false);
                      // Silent when the user dismissed the biometric
                      // prompt: they declined, nothing was removed, and
                      // telling them the action "could not run" reports
                      // their own decision back as a failure.
                      if (isCancelled(e)) return;
                      toast.error("The bulk removal could not run", {
                        description: typeof e === "string" ? e : undefined,
                      });
                    },
                  );
                }}
                disabled={bulkBusy}
                className="rounded bg-[#da3633] px-3 py-1.5 text-sm font-medium text-white hover:bg-[#c93c37] disabled:opacity-50"
              >
                {/* `bulkBusy` existed and nothing read it, so a slow
                    removal showed an unchanged button and a live count
                    ticking down behind the modal. */}
                {bulkBusy
                  ? "Removing…"
                  : `Remove ${safeCount} worktree${safeCount === 1 ? "" : "s"}`}
              </button>
            </div>
          </DialogContent>
        </Dialog>
      ) : null}

      {/* `overflow-hidden` is a BACKSTOP, not the fix (#818).

          The fix is that every cell in a row can now yield; this is what
          happens if some future cell cannot. Without it the border is
          decoration that content walks straight through: the row that
          reported this issue rendered its Remove button and kebab
          outside the box entirely, to the right of the border, where
          they looked like they belonged to nothing. With it, an
          unbounded cell degrades to being clipped at the edge -- still
          wrong, but visibly wrong inside the frame instead of silently
          scattering controls across the page.

          Clipping rather than `overflow-x-auto`: a horizontal
          scrollbar on the list would mean the action buttons can be
          scrolled OUT of view, and the whole complaint was that they
          were unreachable. `rounded-md` also only actually rounds the
          first and last rows' corners once the box clips them. */}
      <div className="overflow-hidden rounded-md border border-[#30363d]">
        {shown.length === 0 ? (
          <div className="px-4 py-12 text-center text-sm text-[#8b949e]">
            No worktrees in this repository.
          </div>
        ) : (
          shown.map((wt) => (
            <Row
              key={wt.path}
              wt={wt}
              repoPath={selected?.path ?? ""}
              pr={prForWorktree(prs, selected?.identity ?? null, wt.branch)}
              assessed={assessed.has(wt.path)}
              onForce={setForcing}
              onUnlock={setUnlocking}
              onRemoveOrphan={runRemoveOrphan}
              onPull={runPull}
              pulling={pullingPath === wt.path}
              onRemove={setPending}
              onClaudify={claudify}
              onForget={forget}
              sizePending={sizing}
              sizeUnmeasurable={wt.sizeUnmeasurable}
              removing={removing === wt.path}
            />
          ))
        )}
      </div>
    </div>
  );
}
