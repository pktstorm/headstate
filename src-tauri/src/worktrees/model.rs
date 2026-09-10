use serde::{Deserialize, Serialize};

/// A checkout with worktrees hanging off it.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Repo {
    /// `owner/repo` from the git remote, when it can be established.
    ///
    /// From the REMOTE, never the directory name -- this app's own
    /// directory is `ghstat` while its repository is
    /// `pktstorm/headstate`. Used to pair a worktree with its pull
    /// request; None means no pairing rather than a fuzzy one.
    #[serde(default)]
    pub identity: Option<String>,
    /// Directory name, e.g. `octo-api`.
    pub name: String,
    /// Absolute path to the main checkout.
    pub path: String,
    pub worktrees: Vec<Worktree>,
    /// When this repository's remote refs were last fetched, RFC 3339,
    /// or `None` if it has never been fetched or the time is unreadable.
    ///
    /// Every merge and upstream verdict below is computed against
    /// `origin/*` refs already on disk -- the scan deliberately never
    /// goes to the network, because a view that opens in a second must
    /// not become one that opens in thirty by fetching 37 remotes.
    ///
    /// That decision is right and stays. What was missing is telling
    /// anyone about it: on this machine one repository's refs were 12
    /// days old, so its rows were answering as of a fortnight ago while
    /// reading like the present tense (#702). `Current` is the worst of
    /// them, because "up to date" is exactly what it does NOT mean.
    ///
    /// `None` rather than a zero or a guess: never fetched and cannot
    /// tell are both "we do not know", and neither is "just now".
    #[serde(default)]
    pub fetched_at: Option<String>,
}

/// Why a worktree can or cannot be removed.
///
/// Deliberately an enum rather than a bool: the UI has to explain ITSELF,
/// and "3 uncommitted files" is actionable where a greyed-out button is
/// not. `NeverPushed` is separate from `Unmerged` because it is the
/// dangerous one -- measured, 5 of 25 sampled worktrees have no upstream
/// at all, so their commits exist nowhere else on earth.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "kind", content = "detail")]
pub enum Safety {
    /// Merged, clean, and pushed. Removable.
    ///
    /// Carries the date the branch landed in the default branch, when it
    /// can be determined -- knowing a branch merged yesterday versus four
    /// months ago changes how confidently you delete it.
    Safe,
    /// The repository's own checkout, not a worktree.
    MainCheckout,
    /// Uncommitted changes; the number of affected paths.
    Dirty(u64),
    /// Commits not on the remote; how many.
    Unpushed(u64),
    /// No upstream branch at all -- nothing has ever been pushed.
    NeverPushed,
    /// Merged, but the remote branch has since been deleted (#732).
    ///
    /// Distinct from `Safe` because the ROUTE to the verdict differs and
    /// the user deserves to see which one they got. `Safe` means the
    /// upstream still exists and agrees; this means the upstream is gone
    /// and the content was found on the default branch instead. Both are
    /// removable, but only one of them can be re-checked against a
    /// remote afterwards.
    ///
    /// Distinct from `NeverPushed` because it is the opposite verdict.
    /// `rev-parse @{u}` fails identically for both, which is exactly the
    /// bug: a branch whose PR merged and whose remote was then deleted
    /// was reported as commits existing only on this machine.
    MergedUpstreamDeleted,
    /// The branch was created and never committed to.
    ///
    /// Its own state rather than a flavour of `Safe` or `NeverPushed`,
    /// because the CLAIM is different and the difference is what the
    /// user came for. `NeverPushed` says "these commits exist only
    /// here", which for a branch with no commits of its own is simply
    /// false -- and the row said it next to "0 commits ahead", a
    /// contradiction one user spent a session resolving by hand.
    /// `Safe` would be true but weaker: it invites "merged when?",
    /// where this answers "there was never anything here".
    Empty,
    /// The repository that owned this worktree is gone.
    ///
    /// A category of its own rather than a flavour of `Unknown`,
    /// because the claim is different: `Unknown` means a check failed
    /// and might succeed later, while this means the checkout can never
    /// be classified again -- there is no git to run in it. It is also
    /// the only state where the DIRECTORY is the whole story, since
    /// nothing else can be read from it.
    Orphaned,
    /// Branch is not merged into the default branch.
    Unmerged,
    /// Someone locked this worktree; git refuses to remove it (#753).
    ///
    /// Carries git's own lock reason, which is the whole point of the
    /// variant. `git worktree lock --reason` exists so the locker can
    /// say who they are, and a tool that locks a tree while it works in
    /// it writes something like "some tool (pid 123)". That string is
    /// the evidence a user needs to tell a LIVE claim from a leftover
    /// one -- a process that is still running versus one that died
    /// without unlocking. `None` when the lock carries no reason, which
    /// git also permits.
    ///
    /// Its own variant rather than a flavour of `Unknown`, because the
    /// check did not fail: the answer is known, specific, and has an
    /// obvious remedy. It is also not a flavour of `Dirty` -- a lock
    /// says nothing about the contents, only that something claimed the
    /// directory.
    ///
    /// NOT removable, and deliberately not force-removable behind the
    /// scenes: `git worktree remove` suggests `-f -f` to override, and
    /// passing that silently would defeat the only mechanism git gives a
    /// concurrent process for saying "I am using this". 13 of 34
    /// worktrees on the reporting machine were locked by running agents.
    Locked(Option<String>),
    /// The directory is gone and git knows the registration is stale.
    ///
    /// Git emits `prunable <reason>` for exactly this, and `git worktree
    /// prune` clears it. Before #753 the missing directory fell through
    /// to `Unknown("directory is missing")`, which reads as corruption
    /// -- a true statement that told the user nothing about what to do,
    /// for what is ordinary, resolvable bookkeeping.
    ///
    /// Carries git's reason (typically "gitdir file points to
    /// non-existent location") rather than the app's own guess, so the
    /// row reports what git actually said.
    ///
    /// NOT removable, though nothing here could be lost: there is no
    /// directory left to remove, so the remove path is simply the wrong
    /// action. Pruning is the right one, and it is a different verb.
    Prunable(String),
    /// Listed, but not yet classified. A transient state the UI shows as
    /// a skeleton rather than as an answer -- distinct from `Unknown`,
    /// which means the check ran and could not decide.
    Pending,
    /// Git could not answer; never assume safe on an error.
    Unknown(String),
}

/// Defaults to `Pending`, never `Safe`.
///
/// A partially-constructed `Worktree` must not be deletable: the default
/// is the value a bug is most likely to leave behind, and neither
/// `Pending` nor `Unknown` is deletable.
impl Default for Safety {
    /// Not-yet-checked, which is NOT the same as checked-and-failed.
    ///
    /// This used to default to `Unknown("not yet classified")`, which the
    /// UI rendered as "could not determine: not yet classified" -- a
    /// failed check, in the same grey as a real failure. The fast listing
    /// lands in ~2.6s and classification takes up to ~57s, so for most of
    /// a minute every row claimed its safety check had failed.
    fn default() -> Self {
        Safety::Pending
    }
}

impl Safety {
    /// Only `Safe` may be deleted. Everything else is disabled in the UI
    /// rather than warned past -- a cleanup tool that occasionally eats a
    /// day of work is worse than no cleanup tool.
    ///
    /// `Empty` is deliberately NOT safe, though nothing on the branch
    /// could be lost. #701 is a report that the REPORTING was wrong --
    /// an empty branch was described as holding commits that exist only
    /// here -- not that the gate was too tight. Making `Empty` safe
    /// would silently promote a large, previously-refused population to
    /// one-click deletable as a side effect of fixing wording: 52 of
    /// 296 worktrees on the reporting machine have no upstream, and an
    /// unknown share of those are empty. Widening the only
    /// unrecoverable action in the app is its own decision, taken on
    /// its own evidence, not a rider on a copy fix.
    ///
    /// The user is not stuck: `Empty` says plainly that there is
    /// nothing to lose, and `remove_worktree_forced` -- reached through
    /// a confirmation that quotes this reason -- is exactly the path
    /// for "the app is being careful and I have read why".
    pub fn is_safe(&self) -> bool {
        // Both arms mean the work is on the default branch and the tree
        // is clean. They are separate variants so the row can say which
        // evidence was used, not because one is safer than the other.
        matches!(self, Safety::Safe | Safety::MergedUpstreamDeleted)
    }

    /// Display-ready prose for the row, so the UI does not re-derive it.
    pub fn reason(&self) -> String {
        match self {
            Safety::Safe => "merged, pushed, safe to delete".into(),
            Safety::MainCheckout => "the repository's main checkout".into(),
            Safety::Dirty(n) => format!("{n} uncommitted file{}", if *n == 1 { "" } else { "s" }),
            Safety::Unpushed(n) => {
                format!("{n} unpushed commit{}", if *n == 1 { "" } else { "s" })
            }
            Safety::NeverPushed => "never pushed — commits exist only here".into(),
            Safety::MergedUpstreamDeleted => "merged; upstream deleted".into(),
            // Says what is TRUE of the branch, not what the app will
            // let you do about it. "Nothing to lose" is the fact the
            // user was trying to establish by hand; whether the Remove
            // button is enabled is a separate, more cautious question
            // answered by `is_safe`.
            Safety::Empty => "no commits of its own — nothing to lose".into(),
            Safety::Unmerged => "branch not merged".into(),
            // Names the locker when git has one. "locked" alone would
            // send the user to the command line to find out by whom;
            // the reason is why `--reason` exists, and it is what
            // separates a live claim from a stale one (#753).
            Safety::Locked(Some(why)) => format!("locked: {why}"),
            Safety::Locked(None) => "locked — no reason given".into(),
            // Says the remedy, because unlike every other refusal here
            // there is one, it is safe, and it is one command. The old
            // wording for this state was "could not determine:
            // directory is missing", which named neither.
            Safety::Prunable(why) => format!("directory is gone — prunable ({why})"),
            Safety::Pending => "checking…".into(),
            // Says what IS known, not what could not be checked. The
            // parent repository is gone, so nothing about this
            // checkout's contents can be established -- and the user
            // needs to know that before deciding, not a hedge.
            Safety::Orphaned => "its repository is gone — nothing here can be checked".into(),
            Safety::Unknown(why) => format!("could not determine: {why}"),
        }
    }
}

/// How a checkout stands against its tracked upstream.
///
/// Separate from `Safety` on purpose: safety answers "may I delete
/// this?", while this answers "is this current?". Folding them together
/// would make the main checkout's row a safety verdict about a directory
/// nobody is proposing to delete.
///
/// Comparison is against the last fetch -- reading refs already on disk,
/// never the network. This is a local disk-usage view, and a scan that
/// silently fetched 37 remotes would be both slow and surprising.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "kind", content = "n")]
pub enum Upstream {
    /// Level with the upstream as of the last fetch.
    Current,
    Ahead(u64),
    Behind(u64),
    /// Both sides moved: `.0` ahead, `.1` behind.
    Diverged(u64, u64),
    /// A local-only branch. Normal, not an error -- and distinctly not
    /// "up to date", which is what a bare zero would imply.
    Untracked,
    /// No branch to compare, so the question does not apply.
    Detached,
    Unknown(String),
}

impl Upstream {
    /// Display-ready prose, so the UI does not re-derive it.
    pub fn reason(&self) -> String {
        let commits = |n: &u64| format!("{n} commit{}", if *n == 1 { "" } else { "s" });
        match self {
            Upstream::Current => "up to date with upstream".into(),
            Upstream::Ahead(n) => format!("{} ahead of upstream", commits(n)),
            Upstream::Behind(n) => format!("{} behind upstream", commits(n)),
            Upstream::Diverged(a, b) => {
                format!("diverged: {} ahead, {} behind", commits(a), commits(b))
            }
            Upstream::Untracked => "no upstream — local only".into(),
            Upstream::Detached => "detached HEAD".into(),
            Upstream::Unknown(why) => format!("upstream unknown: {why}"),
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Worktree {
    pub path: String,
    pub branch: String,
    pub head: String,
    /// Bytes on disk. `None` until measured -- sizing 202 trees is a walk
    /// over hundreds of thousands of files, so it is deliberately lazy.
    pub size_bytes: Option<u64>,
    pub safety: Safety,
    /// True for the repository's own checkout.
    pub is_main: bool,
    /// How this checkout stands against its upstream.
    ///
    /// Computed for EVERY worktree, not just the main checkout. That
    /// restriction was right when a row's only action was Remove and the
    /// safety verdict answered the only question; Claudify changed it,
    /// and "3 commits ahead" is the evidence for whether there is
    /// anything worth keeping.
    pub upstream: Option<Upstream>,
    /// RFC 3339 timestamp of the branch tip's own commit.
    ///
    /// NOT `merged_at`, which is when the work reached the default
    /// branch. A branch written in March and merged in August has both,
    /// and they answer different questions: this one says how stale the
    /// work is, that one says whether it is already accounted for.
    pub last_commit: Option<String>,
    /// `YYYY-MM-DD` when this branch landed in the default branch.
    ///
    /// The date the work reached the default branch, NOT the branch tip's
    /// own commit date. They coincide for a fast-forward but diverge for
    /// a branch written weeks before it merged, and the merge date is the
    /// one that answers "is this safe to forget about".
    pub merged_at: Option<String>,
    /// Git's `locked` line: `Some(reason)`, `Some("")` for a bare lock,
    /// `None` when the worktree is not locked (#753).
    ///
    /// Two levels of Option are not an accident. The OUTER one is the
    /// question "is this locked", and the inner emptiness is "locked,
    /// but the locker left no note" -- git permits `git worktree lock`
    /// with no `--reason`, and emits a bare `locked` line for it.
    /// Collapsing them would make an unlocked worktree and an
    /// unexplained lock the same value, and only one of those refuses
    /// to be removed.
    #[serde(default)]
    pub locked: Option<String>,
    /// Git's `prunable` reason, or `None` when the registration is live.
    ///
    /// No inner Option: git always supplies a reason on this line.
    #[serde(default)]
    pub prunable: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The default must never be deletable. A partially-constructed
    /// `Worktree` is what a bug leaves behind, and this is the one place
    /// where getting it wrong deletes someone's work.
    #[test]
    fn the_default_safety_is_not_deletable() {
        assert!(!Safety::default().is_safe());
        assert_eq!(Safety::default(), Safety::Pending);
    }

    /// `Pending` and `Unknown` are different states and must stay
    /// different.
    ///
    /// `Pending` means "not checked yet" and shows as a skeleton;
    /// `Unknown` means "checked, could not decide" and shows as a
    /// failure. Collapsing them is what made every unclassified row
    /// claim its safety check had failed for the first minute of a scan.
    #[test]
    fn pending_reads_as_waiting_not_as_failure() {
        assert_eq!(Safety::Pending.reason(), "checking…");
        let unknown = Safety::Unknown("git exploded".into());
        assert!(unknown.reason().contains("could not determine"));
        assert_ne!(Safety::Pending.reason(), unknown.reason());
        assert!(!Safety::Pending.is_safe());
    }
}
