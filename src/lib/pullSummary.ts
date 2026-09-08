/// One line for a `git pull --ff-only` result.
///
/// The toast used to show git's stdout verbatim, which for a real
/// fast-forward is the whole diffstat -- hundreds of lines on a busy
/// repository, covering the screen to say "it worked" (#652).
///
/// What it replaced was right about one thing and must stay right about
/// it: git says "Already up to date." when nothing was fetched, and
/// answering that with "Updated" would claim a change that did not
/// happen. So this summarises rather than replaces, and the two answers
/// stay distinguishable.
///
/// Decided from the SHAPE of the output, not from prose. Git's
/// "Already up to date." has been reworded before ("Already up-to-date."
/// before 2.x) and is translated under a non-English locale, so matching
/// it would be a guess that fails quietly in exactly the case this
/// function exists to get right. The summary line is a stable format;
/// its absence is what means nothing arrived.
const SUMMARY = /^\s*(\d+ files? changed.*)$/m;

/// `Updating <old>..<new>` is git's own first line on a fast-forward.
const RANGE = /^Updating ([0-9a-f]+\.\.[0-9a-f]+)$/m;

export function summarisePull(out: string): string {
  const summary = SUMMARY.exec(out);
  if (summary === null) {
    // No diffstat: nothing was fetched. Keep git's own sentence, which
    // is one line and says so better than we would.
    const first = out.trim().split("\n")[0]?.trim();
    return first !== undefined && first !== "" ? first : "Already up to date";
  }
  const range = RANGE.exec(out);
  return range !== null ? `${range[1]} — ${summary[1]}` : summary[1];
}
