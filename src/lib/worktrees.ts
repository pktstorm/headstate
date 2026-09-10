import type { PullRequest, Safety, Upstream, Worktree } from "@/types/pr";

/// Only `safe` may be deleted.
///
/// Everything else is genuinely disabled rather than warned past: a
/// cleanup tool that occasionally eats a day of work is worse than no
/// cleanup tool.
/// The `repo` filter value that selects orphaned worktrees.
///
/// A sentinel rather than a new axis on the store: the sidebar already
/// drives everything through `repo`, and adding a parallel "mode" would
/// mean every consumer had to learn about it. A path-shaped value that
/// no real repository can have keeps the existing plumbing honest.
export const ORPHAN_FILTER = "\0orphaned";

/// Whether a worktree's repository is gone.
///
/// Its own predicate rather than an inline comparison, so the several
/// places that need it cannot drift -- the same reason `isSafe` exists.
export function isOrphaned(s: Safety): boolean {
  return s.kind === "orphaned";
}

export function isSafe(s: Safety): boolean {
  // Both kinds mean the work is on the default branch and the tree is
  // clean, so both are removable (#732). They stay separate kinds so the
  // row can say which evidence was used -- `merged_upstream_deleted`
  // cannot be re-checked against a remote that no longer exists.
  return s.kind === "safe" || s.kind === "merged_upstream_deleted";
}

/// Display-ready prose for a row.
///
/// Mirrors `Safety::reason` on the Rust side. Deliberately duplicated
/// rather than sent over the wire: the wire type is data, and prose in a
/// payload is harder to change than prose in a component.
export function safetyReason(s: Safety): string {
  switch (s.kind) {
    case "safe":
      return "merged, pushed — safe to delete";
    case "main_checkout":
      return "the repository's main checkout";
    case "dirty":
      return `${s.detail} uncommitted file${s.detail === 1 ? "" : "s"}`;
    case "unpushed":
      return `${s.detail} unpushed commit${s.detail === 1 ? "" : "s"}`;
    case "never_pushed":
      return "never pushed — commits exist only here";
    case "merged_upstream_deleted":
      // Says both halves. "Merged" is why it is removable; "upstream
      // deleted" is why the row cannot point at a remote branch to
      // prove it, and is the fact a user comparing this row against
      // GitHub would otherwise find missing.
      return "merged; upstream deleted — safe to delete";
    case "empty":
      // Says what is TRUE of the branch, not what the app will let you
      // do about it: the Remove button stays disabled, deliberately,
      // but the row must stop claiming commits that do not exist.
      return "no commits of its own — nothing to lose";
    case "unmerged":
      return "branch not merged";
    case "locked":
      // Names the locker, because that is the fact the user acts on:
      // "some tool (pid 123)" answers whether the claim is live or
      // left behind by a process that died, and a bare "locked" would
      // send them to a terminal to find out (#753).
      return s.detail === null
        ? "locked — no reason given"
        : `locked: ${s.detail}`;
    case "prunable":
      // Says the remedy. Unlike the other refusals there IS one, it is
      // safe, and it is one command -- where the old wording for this
      // state, "could not determine: directory is missing", named
      // neither the cause nor the cure.
      return `directory is gone — prunable (${s.detail})`;
    case "pending":
      return "checking…";
    case "orphaned":
      // Says what IS known rather than hedging: the parent repository
      // is gone, so nothing about the contents can be established, and
      // the user needs that before deciding.
      return "its repository is gone — nothing here can be checked";
    default:
      return `could not determine: ${s.detail}`;
  }
}

/// What the force-removal confirmation warns about, for one safety
/// state.
///
/// Three cases, not two, because #701 showed what the missing one
/// costs. `never_pushed` names the specific loss -- commits that exist
/// nowhere else. `empty` has no loss to name, so it says so plainly
/// rather than inheriting a warning about commits it does not have;
/// that sentence is the whole reason this state exists. Everything else
/// gets the general form, which is honest about the app's uncertainty
/// without inventing a danger.
///
/// Every branch still ends in "this cannot be undone": the directory
/// goes either way, and the user is one click from it.
export function forceWarning(s: Safety): string {
  switch (s.kind) {
    case "never_pushed":
      return "These commits are not pushed anywhere. This cannot be undone.";
    case "empty":
      return "This branch has no commits of its own, so nothing on it would be lost. Removing the directory cannot be undone.";
    case "locked":
      // Says the truth the general wording would hide: forcing here
      // does not work. `remove_worktree_forced` relaxes Headstate's
      // gate but still calls git WITHOUT `--force`, and git refuses a
      // locked tree on its own account -- so the user would confirm a
      // destructive-sounding dialog and get an error. Naming the
      // unlock is not an invitation to ignore the lock; it is the only
      // route that exists, and the reason is quoted beside it (#753).
      return "This worktree is locked, and git will refuse to remove it until it is unlocked — check the lock reason above first, in case the process that set it is still running.";
    case "prunable":
      // There is no directory to remove, so the destructive framing is
      // simply wrong here. Nothing can be lost and nothing will be.
      return "This worktree's directory is already gone; only the stale registration remains. Removing it loses nothing, but `git worktree prune` is the command that clears it.";
    default:
      return "Headstate does not consider this safe to remove. This cannot be undone.";
  }
}

/// The open pull request for a worktree, if there is one.
///
/// Keyed on repo AND branch, never branch alone: branch names are not
/// unique across repositories -- this account has `feat/egr33-*` in two
/// of them -- and repo identity comes from the git remote rather than
/// the directory name.
///
/// DISPLAY ONLY. This must never widen a safety gate: a wrong pairing
/// would attach GitHub's authoritative-looking "merged" to the wrong
/// directory, and deletion is the one unrecoverable action here.
export function prForWorktree(
  prs: PullRequest[],
  repoIdentity: string | null,
  branch: string,
): PullRequest | null {
  if (!repoIdentity || !branch) return null;
  return (
    prs.find((p) => p.repo === repoIdentity && p.head_ref === branch) ?? null
  );
}

/// Whether this row can be handed to a coding agent.
///
/// Every state that is NOT removable and not the main checkout. That is
/// 124 of 268 worktrees on a real machine, and the largest group --
/// never-pushed, at 52 -- is the one where the question matters most,
/// because those commits exist nowhere else.
///
/// `pending` is excluded on purpose: offering an action based on a
/// safety verdict that has not arrived is the bug #190 was.
///
/// `locked` and `prunable` are excluded too, and they are the states
/// where the blanket rule "not removable and not the main checkout"
/// stops fitting (#753). The action copies a prompt asking a coding
/// agent to assess the worktree's CONTENT, and neither of these is a
/// question about content:
///
/// - `prunable` has no directory left. There is nothing on disk for an
///   agent to open, so the prompt would send it somewhere that does
///   not exist.
/// - `locked` is a claim by another process -- very often an agent
///   already working in that tree, which is precisely how the
///   reporting machine acquired 13 of them. Pointing a second agent at
///   a directory the first has locked is the one thing the lock exists
///   to prevent. The lock is also not a property of the branch, so
///   once it is cleared the row reports its real state and the action
///   returns on its own.
export function canClaudify(s: Safety): boolean {
  return (
    s.kind === "unmerged" ||
    s.kind === "never_pushed" ||
    s.kind === "unpushed" ||
    s.kind === "dirty" ||
    // Included, because the rule above is "not removable and not the
    // main checkout" and `empty` is both. It is a weaker case than the
    // others -- there is nothing on the branch to assess -- but the
    // gate keeps `empty` un-removable, so withholding the assess action
    // too would leave the row with no action at all, which is precisely
    // the dead end this list exists to avoid.
    s.kind === "empty"
  );
}

/// The final component of a path, whichever separator it uses.
///
/// `split("/")` returns the whole string unchanged on a Windows path, so
/// every row would show `C:\Users\me\code\proj-feature` instead of
/// `proj-feature`. Splitting on both separators is enough here: a
/// directory name cannot itself contain either one.
export function pathBasename(path: string): string {
  const parts = path.split(/[\\/]/).filter(Boolean);
  return parts[parts.length - 1] ?? path;
}

/// Whether a row is still waiting on its safety check.
///
/// The fast listing lands in ~2.6s and classification takes up to ~57s,
/// so this is most of the first minute on a large tree -- long enough
/// that the row must say "still working" rather than show a value that
/// reads as final.
export function isPending(s: Safety): boolean {
  return s.kind === "pending";
}

/// Display-ready prose for a checkout's upstream state.
///
/// Mirrors `Upstream::reason` on the Rust side, for the same reason
/// `safetyReason` does.
///
/// Says "as of last fetch" on the states where staleness changes the
/// meaning: "behind 40" invites a pull, and the user should know the
/// number could itself be stale. "Up to date" carries the same caveat
/// and is the one most likely to mislead.
export function upstreamReason(u: Upstream): string {
  const commits = (n: number) => `${n} commit${n === 1 ? "" : "s"}`;
  switch (u.kind) {
    case "current":
      return "up to date with upstream (as of last fetch)";
    case "ahead":
      return `${commits(u.n)} ahead of upstream`;
    case "behind":
      return `${commits(u.n)} behind upstream (as of last fetch)`;
    case "diverged":
      return `diverged — ${commits(u.n[0])} ahead, ${commits(u.n[1])} behind`;
    case "untracked":
      return "no upstream — local only";
    case "detached":
      return "detached HEAD";
    default:
      return `upstream unknown: ${u.n}`;
  }
}

/// A compact ahead/behind for a dense worktree row.
///
/// The long form ("3 commits behind upstream (as of last fetch)") is
/// right for the main checkout's own line, where it is the only thing
/// said. A row already carries name, branch, safety, date, and size, so
/// this is the arrow notation git users already read.
///
/// Returns null when there is nothing worth saying: an up-to-date branch
/// adds noise, not information.
export function upstreamShort(u: Upstream): string | null {
  switch (u.kind) {
    case "ahead":
      return `↑${u.n}`;
    case "behind":
      return `↓${u.n}`;
    case "diverged":
      return `↑${u.n[0]} ↓${u.n[1]}`;
    // Local-only is worth saying: it is the difference between "this is
    // redundant" and "this is the only copy".
    case "untracked":
      return "local only";
    default:
      return null;
  }
}

/// Tailwind colour for an upstream state.
///
/// Green only for genuinely current. Amber where action might be wanted,
/// grey where the question simply does not apply -- a local-only repo is
/// not a problem, it is a choice.
export function upstreamTone(u: Upstream): string {
  switch (u.kind) {
    case "current":
      return "text-[#3fb950]";
    case "behind":
    case "diverged":
      return "text-[#d29922]";
    default:
      return "text-[#8b949e]";
  }
}

/// Tailwind colour for a safety state.
///
/// Green ONLY for safe. Amber for states that hold work you might still
/// want; grey for the main checkout, which is not a problem at all.
export function safetyTone(s: Safety): string {
  switch (s.kind) {
    // Green for both merged states: the button is enabled for each, and
    // a different colour would imply a different degree of safety
    // rather than a different route to the same verdict (#732).
    case "safe":
    case "merged_upstream_deleted":
      return "text-[#3fb950]";
    case "main_checkout":
      return "text-[#8b949e]";
    case "pending":
      return "text-[#8b949e]";
    case "never_pushed":
      return "text-[#f85149]";
    case "empty":
      // Grey, and explicitly so rather than by falling through to the
      // default. This is the one colour on the row that separates
      // "nothing here" from the red beside it -- an empty branch put in
      // red would repeat #701's mistake in a different medium, telling
      // the user work is at risk when there is no work. Not green
      // either: green means one-click removable, and it is not.
      return "text-[#8b949e]";
    case "dirty":
    case "unpushed":
      return "text-[#d29922]";
    case "locked":
      // Amber, alongside the other "you can act on this" states. Not
      // red: a lock endangers nothing -- it is a claim by another
      // process, and the worst case of ignoring it is that the row
      // stays. Not grey either, because unlike `empty` this is an
      // obstacle the user may well want to clear, and on the reporting
      // machine it covers a third of the rows (#753).
      return "text-[#d29922]";
    case "prunable":
      // Grey. There is no directory left, so there is nothing at risk
      // and nothing to reclaim -- it is a bookkeeping entry, and amber
      // would ask for attention that the row does not deserve.
      return "text-[#8b949e]";
    case "orphaned":
      // Amber, not red: an orphan is not dangerous, it is
      // UNVERIFIABLE. Red would put it beside "commits exist only
      // here" and imply work is at risk; grey would let 2.5 GB of
      // unreachable directories read as ordinary.
      return "text-[#d29922]";
    default:
      return "text-[#8b949e]";
  }
}

/// Bytes as a short human string. `null` renders as an em dash rather
/// than "0 B", which would claim a measurement that has not happened.
export function formatSize(bytes: number | null): string {
  if (bytes === null) return "—";
  const units = ["B", "KB", "MB", "GB", "TB"];
  let n = bytes;
  let i = 0;
  while (n >= 1024 && i < units.length - 1) {
    n /= 1024;
    i += 1;
  }
  return `${n < 10 && i > 0 ? n.toFixed(1) : Math.round(n)} ${units[i]}`;
}

/// What a worktree's pull request says is wrong, or null if nothing is.
///
/// `prForWorktree` already resolves the pull request for a row, but the
/// row showed only its NUMBER -- while the `PullRequest` in hand carries
/// `ci`, `merge`, and `review`. That is usually the reason to come back
/// to a worktree at all: its pull request broke.
///
/// Ordered by loudness, and returns only ONE: a row is a glance, not a
/// report, and three stacked badges would push the path off the line.
/// Red CI wins over conflicts because it is the more surprising of the
/// two -- a conflict is a consequence of other work landing, a red build
/// is a consequence of yours.
///
/// Silent on a healthy pull request, and silent while CI is still
/// RUNNING: pending is an unfinished answer, not a problem, and a badge
/// on every row is noise. The number stays a link for anyone who wants
/// the detail. No new fetching: every field here is already on the
/// PullRequest this function is handed.
export function worktreeSignal(
  // `undefined` as well as `null`: the prop is optional on the row, and
  // both mean the same thing here -- no pull request, nothing to say.
  pr: PullRequest | null | undefined,
): { label: string; className: string } | null {
  if (!pr) return null;
  if (pr.ci === "failure") return { label: "CI failing", className: "text-[#f85149]" };
  if (pr.merge === "conflicted") return { label: "Conflicts", className: "text-[#f85149]" };
  if (pr.review === "changes_requested") {
    return { label: "Changes requested", className: "text-[#d29922]" };
  }
  return null;
}

/// The total size of a set of worktrees, distinguishing "not measured
/// yet" from "measured, and it is zero".
///
/// The bug this replaces summed with `?? 0` and then wrote `|| null`:
///
///     .reduce((n, w) => n + (w.size_bytes ?? 0), 0) || null
///
/// `0` is falsy, so a genuine zero became null and rendered as `—`.
/// Worse, an unmeasured set also summed to 0 and rendered the same
/// way, so "still measuring" and "nothing to reclaim" were one symbol.
/// Sizing is the slow half of the scan, which is why the dash appeared
/// only sometimes -- it depended on whether sizing had caught up.
///
/// Returns null ONLY when nothing in the set has a known size.
export function totalSize(items: { size_bytes: number | null }[]): number | null {
  const measured = items.filter((w) => w.size_bytes !== null);
  if (measured.length === 0) return null;
  return measured.reduce((n, w) => n + (w.size_bytes ?? 0), 0);
}

/// How old the answers on a repository's rows are, as prose, or `null`
/// when they are fresh enough not to mention.
///
/// Every merge and upstream verdict is computed against `origin/*` refs
/// already on disk -- the scan never goes to the network, deliberately,
/// because fetching every remote would turn a one-second view into a
/// thirty-second one. What was missing is saying so: on one machine a
/// repository's refs were 12 days old while its rows read like the
/// present tense (#702).
///
/// Silent below a day. Under that the note is noise -- a fetch this
/// morning is not a caveat -- and a caveat shown always is a caveat
/// nobody reads. `null` fetch time is NOT silent: never fetched is the
/// stalest state there is, not the freshest.
export function refAge(fetchedAt: string | null, now = new Date()): string | null {
  if (fetchedAt === null) return "never fetched";
  const at = Date.parse(fetchedAt);
  if (Number.isNaN(at)) return "never fetched";
  const days = Math.floor((now.getTime() - at) / 86_400_000);
  if (days < 1) return null;
  return `as of a fetch ${days} day${days === 1 ? "" : "s"} ago`;
}

/// The orderings the worktree list offers (#771).
///
/// Three axes because three are already on every row -- name, size and
/// age -- and no new data is needed for any of them. The page exists to
/// answer "which of these is biggest" on a repository with 100+
/// worktrees, and until now that question could only be answered by
/// reading every row.
///
/// Each axis is bidirectional. Size descending is what reclaims disk;
/// size ascending finds the scratch trees worth clearing wholesale. Age
/// descending surfaces the safe wins -- a worktree last touched four
/// months ago is a far easier delete than one from this morning,
/// whatever the safety verdict says.
export type WorktreeSort =
  | "name-asc"
  | "name-desc"
  | "size-desc"
  | "size-asc"
  | "age-desc"
  | "age-asc";

/// What the sort control offers, in the order it offers it.
///
/// Prose rather than a bare column name, the same choice `ArtifactsPage`
/// made with "Least recently written": "oldest" invites reading the age
/// as a creation date, when what is actually ordered is the last commit.
///
/// Size leads because it is the question the view exists for.
export const WORKTREE_SORT_LABELS: Record<WorktreeSort, string> = {
  "size-desc": "Largest first",
  "size-asc": "Smallest first",
  "age-desc": "Least recently committed",
  "age-asc": "Most recently committed",
  "name-asc": "Name (A–Z)",
  "name-desc": "Name (Z–A)",
};

/// Order one repository's worktree rows.
///
/// Two invariants survive every sort, because neither is a preference:
///
/// The MAIN CHECKOUT is always first. It is not a peer of the rows below
/// it -- every one of those is a removal candidate and it never is, so
/// sorting it among them invites reading it as one. Its row also carries
/// the upstream prose ("behind by 40"), which is the reason the
/// worktrees under it are stale, and an explanation belongs above the
/// thing it explains.
///
/// ASSESSED rows come next. The user has just come back from reading a
/// verdict, and finding that row again among 124 candidates is the part
/// that made this flow feel unfinished. A size sort must not bury the
/// one row they were mid-decision on.
///
/// UNKNOWN VALUES SORT LAST, in both directions, and that is the whole
/// reason this comparator is not a one-liner. Sizes arrive
/// asynchronously (#754, #758) and can be genuinely absent -- still
/// pending, or measured and failed. Treating a missing size as zero
/// would rank every unmeasured row as the smallest on the page, which
/// under "Largest first" hides exactly the directories that might be
/// huge. That is the ordering bug #360 describes, and putting unknowns
/// last is the answer `ArtifactsPage` already reached.
///
/// Last means last in BOTH directions. Under "Smallest first" an
/// unknown is still not a claim of 0 B, so it does not get to lead
/// either -- an unknown is absent from the ordering, not an extreme of
/// it. Ties, including ties among unknowns, fall back to the path, so
/// the result is stable rather than arbitrary.
///
/// Name never has this problem: a worktree always has a path. That
/// makes a name sort the one ordering fully known at first render, and
/// a genuine escape hatch while sizes are still landing.
export function sortWorktrees<T extends Worktree>(
  worktrees: T[],
  sort: WorktreeSort,
  assessed: ReadonlySet<string> = new Set(),
): T[] {
  /// The value this row offers on the chosen axis, or null when it
  /// cannot answer. Age is the last commit as epoch milliseconds; an
  /// absent or unparseable timestamp is null rather than 0, which would
  /// date the row to 1970 and pin it to one end of the list.
  const key = (w: T): number | null => {
    if (sort === "size-asc" || sort === "size-desc") return w.size_bytes ?? null;
    if (sort === "age-asc" || sort === "age-desc") {
      if (!w.last_commit) return null;
      const at = Date.parse(w.last_commit);
      return Number.isNaN(at) ? null : at;
    }
    return null;
  };
  // A larger epoch is MORE recent, so "least recently committed" wants
  // the SMALLEST number first -- the inverse of size, where "largest
  // first" wants the biggest. Spelled out rather than folded into the
  // comparator: getting it backwards yields a list that looks plausibly
  // sorted and is exactly wrong.
  const biggestFirst = sort === "size-desc" || sort === "age-asc";

  return [...worktrees].sort((a, b) => {
    if (a.is_main !== b.is_main) return a.is_main ? -1 : 1;
    const aa = assessed.has(a.path) ? 0 : 1;
    const bb = assessed.has(b.path) ? 0 : 1;
    if (aa !== bb) return aa - bb;

    if (sort === "name-asc" || sort === "name-desc") {
      // The basename, not the whole path: it is what the row shows, and
      // ordering by a prefix every row in a repository shares would be
      // ordering by nothing.
      const cmp = pathBasename(a.path).localeCompare(pathBasename(b.path));
      return sort === "name-asc" ? cmp : -cmp;
    }

    const va = key(a);
    const vb = key(b);
    if (va === null && vb === null) return a.path.localeCompare(b.path);
    if (va === null) return 1;
    if (vb === null) return -1;
    if (va === vb) return a.path.localeCompare(b.path);
    return biggestFirst ? vb - va : va - vb;
  });
}
