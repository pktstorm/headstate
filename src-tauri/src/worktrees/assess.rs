//! Facts about a worktree's work, for handing to a coding agent.
//!
//! Gathered on demand rather than during the scan: this is six git calls
//! per worktree, and the scan already walks 268 of them. Nobody needs
//! these numbers until they click.
//!
//! Everything here is local. No network, no GitHub API -- the comparison
//! is against the `origin/main` ref already on disk, so this works on a
//! plane and costs no rate limit.
//!
//! # Why this module still does not fetch (#815)
//!
//! That local-only design is the direct cause of the bug #815 reports.
//! The numbers below are computed against whatever `origin/main` on disk
//! happens to say, so an agent handed them would assess a branch as
//! unmerged work worth finishing when every commit in it had already
//! landed upstream -- then do the work, and find out at PR time. #702
//! measured the staleness that makes this happen: one repository on this
//! machine had refs 12 days old while its rows read like the present
//! tense.
//!
//! The fix is in the PROMPT, not here. `prompt()` now names the base ref
//! explicitly, tells the agent to refresh `origin` before believing
//! anything, and tells it to stop and report rather than work if the
//! branch is already upstream. Three reasons that is the better half to
//! change:
//!
//! 1. The agent's own refresh is the only one that is actually fresh.
//!    Claudify copies a command to the clipboard; the user pastes it into
//!    a shell seconds, minutes, or a day later. A refresh at copy time is
//!    stale again by the time the agent reads its own prompt, so the
//!    agent has to do one regardless -- and then the click's bought
//!    nothing but latency.
//! 2. `git()` is documented as local-only and bounded at 30s, a bound
//!    chosen for a stalled FILESYSTEM. A network refresh over a slow link
//!    can legitimately exceed 30s, and every field here turns a git
//!    failure into an ABSENT fact -- so a slow network would silently
//!    produce an assessment that states nothing, which is precisely the
//!    failure mode the `Option` fields exist to prevent.
//! 3. A network call inside one row's click would move every OTHER row's
//!    merge verdict on the page, with no visible cause. `size_worktrees`
//!    set the precedent that an expensive operation lives behind its own
//!    explicit affordance rather than riding along on an unrelated one.
//!
//! Rejected, with reasons:
//!
//! - **On the render path** (the scan, or `assess_worktree`). An implicit
//!   network call per page load across 37 remotes is what `Repo`'s
//!   `fetched_at` doc already declines: "a view that opens in a second
//!   must not become one that opens in thirty".
//! - **Once per repo inside the Claudify click.** Defensible -- it is a
//!   deliberate gesture already spending six git calls -- and it is what
//!   #815 leans towards as a second best. Declined on (1): the agent
//!   refreshes regardless, so this is a duplicate network round trip
//!   whose only visible effect is making the click feel broken on a slow
//!   link. `claudify_command` is also a SYNCHRONOUS Tauri command, unlike
//!   `assess_worktree`'s `spawn_blocking`, so an unbounded network call
//!   there blocks the IPC thread rather than a pool thread.
//! - **Refuse to produce a prompt when the refs are stale.** Withholding
//!   the feature is worse than carrying the caveat: the agent can refresh
//!   the refs itself, which is exactly what it is now told to do.
//!
//! What this module does instead is SAY how stale it is. `fetched_at`
//! rides along in the struct and `prompt()` prints it, so the numbers are
//! labelled as-of-a-refresh rather than presented as live.

use super::scan::{default_branch, fetched_at, git};
use serde::Serialize;
use std::path::Path;

/// How many commit subjects to carry. A branch with 200 commits should
/// not produce a prompt nobody can read; ten is enough to see the shape
/// of the work, and the count says what was elided.
const MAX_SUBJECTS: usize = 10;

/// What the assessment prompt states as fact.
///
/// Every field is `Option` or defaulted because a git call can fail, and
/// a prompt that asserts "0 commits ahead" when the check failed sends an
/// agent looking for work that is there. Absent means absent.
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct Assessment {
    pub path: String,
    pub branch: String,
    /// Commits on this branch that are not on the default branch.
    pub commits_ahead: Option<u64>,
    pub files_changed: Option<u64>,
    pub insertions: Option<u64>,
    pub deletions: Option<u64>,
    /// Relative, as git prints it: "3 weeks ago".
    pub last_activity: Option<String>,
    /// Never pushed means these commits exist only on this machine. The
    /// single most important fact in the whole struct.
    pub has_upstream: bool,
    /// Newest first, capped at `MAX_SUBJECTS`.
    pub subjects: Vec<String>,
    /// How many subjects were elided by the cap.
    pub subjects_elided: u64,
    /// Uncommitted paths. Not covered by the diff against the default
    /// branch, so an agent would otherwise assess an incomplete picture.
    pub uncommitted: u64,
    /// The ref every count above was measured against, by name --
    /// `origin/main` on a repository with a remote, a bare `main` on a
    /// purely local one (#815).
    ///
    /// Carried rather than re-derived, and NAMED rather than described.
    /// The prompt used to say "the default branch", which is ambiguous
    /// in exactly the way that caused the bug: an agent reading it is
    /// free to resolve it as the local `main`, the merge-base the
    /// worktree was cut from, or the remote ref -- three different
    /// answers, one of which is the one the numbers actually came from.
    /// Telling the agent the literal ref makes its own `rev-list`
    /// reproduce these counts instead of inventing different ones.
    ///
    /// `String`, not `Option`: `default_branch` always answers, falling
    /// back to the local branch name, so there is no "we could not tell"
    /// state to represent here.
    pub base: String,
    /// When this repository's remote refs were last refreshed, RFC 3339,
    /// or `None` for never/unreadable.
    ///
    /// The same `FETCH_HEAD` mtime the Worktrees page shows (#702), but
    /// plumbed through to the PROMPT, which is where it was missing
    /// (#815). The page's caveat never reached the agent's context, so
    /// the agent had no way to know its inputs were as-of-a-refresh
    /// rather than live.
    ///
    /// `None` means "we do not know", never "just now" -- never
    /// refreshed is the stalest state there is, and the prompt says so
    /// in those words.
    pub fetched_at: Option<String>,
}

/// Gather what can be known about a worktree's work, locally.
pub fn assess(repo_path: &str, worktree_path: &str, branch: &str) -> Assessment {
    let repo = Path::new(repo_path);
    let dir = Path::new(worktree_path);
    // NOT `origin/{}`. `default_branch` now returns the remote-tracking
    // ref itself (#757), so re-prefixing here would ask git for
    // `origin/origin/main` and every count, diff and log below would
    // come back absent -- an assessment that silently states nothing.
    // The one repository shape that loses the remote is the purely-local
    // one, where `default_branch` falls back to the local branch and
    // this comparison is against the only ref there is.
    let base = default_branch(repo);

    // Three dots: the merge-base comparison, which shows what this BRANCH
    // added. Two dots would fold in everything the default branch gained
    // since, inflating an old branch's diff with unrelated work and
    // sending the agent to assess changes that are not the user's.
    let range3 = format!("{base}...{branch}");
    let range2 = format!("{base}..{branch}");

    let count = |args: &[&str]| -> Option<u64> { git(dir, args).ok()?.trim().parse::<u64>().ok() };

    let mut a = Assessment {
        path: worktree_path.to_string(),
        branch: branch.to_string(),
        // Both read from the MAIN checkout, not the worktree: `base` is a
        // property of the repository, and `FETCH_HEAD` lives in the
        // repository's `.git` directory rather than in a worktree's
        // `.git` file.
        base: base.clone(),
        fetched_at: fetched_at(repo),
        commits_ahead: count(&["rev-list", "--count", &range2]),
        // `--` before the ref: a branch named `--output=/path` would
        // otherwise make git write to an arbitrary file. The boundary in
        // `parse_porcelain` should already have dropped it; this is the
        // second layer.
        last_activity: git(dir, &["log", "-1", "--format=%cr", "--", branch])
            .ok()
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty()),
        has_upstream: git(dir, &["rev-parse", "--abbrev-ref", "@{u}"]).is_ok(),
        ..Default::default()
    };

    if let Ok(stat) = git(dir, &["diff", "--shortstat", &range3]) {
        let (f, i, d) = parse_shortstat(&stat);
        a.files_changed = f;
        a.insertions = i;
        a.deletions = d;
    }

    if let Ok(log) = git(dir, &["log", &range2, "--format=%s"]) {
        let all: Vec<String> = log
            .lines()
            .map(str::trim)
            .filter(|l| !l.is_empty())
            .map(str::to_string)
            .collect();
        a.subjects_elided = all.len().saturating_sub(MAX_SUBJECTS) as u64;
        a.subjects = all.into_iter().take(MAX_SUBJECTS).collect();
    }

    if let Ok(status) = git(dir, &["status", "--porcelain"]) {
        a.uncommitted = status.lines().filter(|l| !l.trim().is_empty()).count() as u64;
    }

    a
}

impl Assessment {
    /// The prompt handed to a coding agent.
    ///
    /// States what is known and asks four questions, in the order a human
    /// would: what is this, what would it break, is it finished, and what
    /// should I do. The agent is told explicitly NOT to push or open a
    /// pull request -- a button labelled "assess" that writes to GitHub is
    /// a different feature with a different risk profile.
    ///
    /// A fact that could not be gathered is OMITTED rather than guessed.
    /// "0 commits ahead" when the check failed would send an agent looking
    /// for work that is there.
    ///
    /// # Refresh first, and say how stale these numbers are (#815)
    ///
    /// Two things this prompt used to get wrong, which together produced
    /// the same wasted-work loop over and over: an agent assessed a
    /// branch as unfinished work, finished it, opened a PR, and found
    /// every commit already upstream.
    ///
    /// **It said "the default branch", twice, unqualified.** Three
    /// different refs answer to that phrase -- the local `main`, the
    /// merge-base the worktree was cut from, and `origin/main` -- and
    /// only the last is what the numbers above were measured against. So
    /// the prompt now interpolates `base` literally. An agent that runs
    /// `rev-list --count origin/main..branch` reproduces the count it was
    /// handed; one that guesses `main` does not, and has no way to know
    /// it diverged.
    ///
    /// **It never said to refresh.** Nothing on this path fetches (see
    /// the module header for why that stays true), so the counts are as
    /// old as the last refresh -- 12 days, measured (#702). The agent is
    /// now told to refresh `origin` and to re-derive the comparison
    /// itself, which is the whole fix: even when the app's inputs are
    /// stale, the agent's are not, because it refreshed them.
    ///
    /// The already-upstream check is ordered BEFORE the four questions
    /// and phrased as a stop, not a caveat. Order is the entire point.
    /// "Check whether this is already merged" placed among the questions
    /// gets answered in a report written after the work is done, which is
    /// the exact failure being fixed; a STOP placed first is the only
    /// form of it that saves anything.
    ///
    /// The refresh is worded as `git fetch origin` with no refspec and no
    /// `--prune`. A refspec would need the default branch's short name,
    /// which `base` carries only as `origin/<name>`, and pruning is a
    /// write to the user's refs that an assessment was not asked to make.
    ///
    /// Staleness is stated as the timestamp rather than as prose ("2
    /// hours ago"): the prompt may be pasted a day after it was copied,
    /// and a relative phrase computed at copy time would then be a lie
    /// with no way for the reader to notice. An absolute instant stays
    /// true however long the clipboard holds it.
    pub fn prompt(&self) -> String {
        let mut out = format!(
            "Assess the git worktree at {}, branch {}.\n\n",
            self.path, self.branch
        );

        // Ordered first and labelled as a precondition, because an agent
        // that reads the facts below before this line has already started
        // to believe them.
        out.push_str(&format!(
            "FIRST, before assessing anything:\n\
             \x20 - Run `git fetch origin` in that worktree. The numbers below were \
             measured against the on-disk `{base}` ref, and nothing in this app \
             fetches, so they can be days old.\n\
             \x20 - Re-derive the comparison yourself against `{base}` \
             (`git rev-list --count {base}..{branch}`). Use `{base}` by that name, \
             not a local `main` and not the merge-base this worktree was cut from; \
             they are different refs and can give different answers.\n\
             \x20 - If the work on {branch} is ALREADY on `{base}` -- merged, \
             rebased, cherry-picked, or landed under other commit hashes -- STOP. \
             Report that it is already upstream and do no further work. Do not \
             finish, tidy, rebase or prepare the branch first. That wasted-work \
             path is the reason this instruction exists.\n\n",
            base = self.base,
            branch = self.branch,
        ));

        // Stated even when the refresh above will supersede it: the agent
        // reads the facts whether or not it runs the commands, and an
        // unlabelled number reads as current.
        out.push_str(&match &self.fetched_at {
            Some(at) => format!("These numbers are as of a fetch at {at}:\n\n"),
            // The stalest state there is, not the freshest -- the same
            // rule `refAge` follows on the page (#702).
            None => "These numbers come from refs that have NEVER been fetched:\n\n".to_string(),
        });

        let mut facts = Vec::new();
        if let Some(n) = self.commits_ahead {
            facts.push(format!("  {n} commit{} ahead of {}", plural(n), self.base));
        }
        if let Some(f) = self.files_changed {
            let ins = self.insertions.unwrap_or(0);
            let del = self.deletions.unwrap_or(0);
            facts.push(format!("  {f} file{} changed, +{ins}/-{del}", plural(f)));
        }
        if let Some(when) = &self.last_activity {
            facts.push(format!("  last commit {when}"));
        }
        if self.uncommitted > 0 {
            // Not covered by the diff against the base ref, so an agent
            // would otherwise assess an incomplete picture.
            facts.push(format!(
                "  {} uncommitted file{} in the working tree",
                self.uncommitted,
                plural(self.uncommitted)
            ));
        }
        if !self.has_upstream {
            // The single most important line here, so it goes last where
            // it is read rather than buried among the counts.
            facts.push("  NOT PUSHED -- these commits exist only on this machine".to_string());
        }
        if !facts.is_empty() {
            out.push_str(&facts.join("\n"));
            out.push_str("\n\n");
        }

        if !self.subjects.is_empty() {
            out.push_str("Commits:\n");
            for s in &self.subjects {
                out.push_str(&format!("  - {s}\n"));
            }
            if self.subjects_elided > 0 {
                out.push_str(&format!("  ... and {} more\n", self.subjects_elided));
            }
            out.push('\n');
        }

        // `base` again rather than "the default branch": question 2 is
        // the one whose answer changes with the ref, so leaving it vague
        // here would reintroduce the ambiguity the FIRST block removes.
        //
        // "safe to discard" stays available as a recommendation even
        // though the already-upstream case now stops above: a branch can
        // be discardable for reasons that have nothing to do with having
        // landed -- abandoned, superseded, an experiment.
        out.push_str(&format!(
            "Then please:\n\
             \x20 1. What does this change, and why does it appear to exist?\n\
             \x20 2. What would it affect if merged into {base}?\n\
             \x20 3. Is it complete, or was it abandoned midway?\n\
             \x20 4. Recommend one: open a PR / needs work first / safe to discard \
             / already upstream.\n\n\
             Do not push anything or open a pull request. Report your assessment \
             and let me decide.",
            base = self.base,
        ));
        out
    }

    /// The whole command, ready to paste into a shell.
    ///
    /// Single quotes around both the path and the prompt: that is what
    /// makes `$VARS`, backticks, and `;` in a branch name inert rather
    /// than executable. Embedded single quotes are closed, escaped, and
    /// reopened -- the only way to get one inside a single-quoted string.
    pub fn command(&self, claude: &str) -> String {
        format!(
            "cd {} && {claude} {}",
            shell_quote(&self.path),
            shell_quote(&self.prompt())
        )
    }
}

/// Wrap in single quotes so the shell treats the contents as literal.
///
/// A worktree path or commit subject is untrusted input as far as the
/// shell is concerned: a branch named `x'; rm -rf ~` would otherwise
/// paste as two commands.
fn shell_quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', r"'\''"))
}

fn plural(n: u64) -> &'static str {
    if n == 1 {
        ""
    } else {
        "s"
    }
}

/// Pull the three numbers out of `git diff --shortstat`.
///
/// The line omits a clause entirely when it is zero -- a pure-addition
/// diff has no "deletions" part at all -- so this reads each independently
/// rather than positionally.
fn parse_shortstat(s: &str) -> (Option<u64>, Option<u64>, Option<u64>) {
    let num_before = |needle: &str| -> Option<u64> {
        let idx = s.find(needle)?;
        s[..idx]
            .split_whitespace()
            .next_back()
            .and_then(|n| n.parse().ok())
    };
    (
        num_before("file"),
        num_before("insertion"),
        num_before("deletion"),
    )
}

#[cfg(test)]
mod live {
    use super::*;

    /// Against a real worktree on this machine. Run manually:
    /// `cargo test --lib live_assess -- --ignored --nocapture`
    #[test]
    #[ignore]
    fn live_assess_a_real_worktree() {
        // Supplied by the environment rather than hardcoded. The paths
        // that used to sit here named a private project, which is both a
        // leak and useless to anyone else running this.
        let (repo, wt, branch) = match (
            std::env::var("HEADSTATE_LIVE_REPO"),
            std::env::var("HEADSTATE_LIVE_WORKTREE"),
            std::env::var("HEADSTATE_LIVE_BRANCH"),
        ) {
            (Ok(r), Ok(w), Ok(b)) => (r, w, b),
            _ => {
                println!(
                    "set HEADSTATE_LIVE_REPO, HEADSTATE_LIVE_WORKTREE and \
                     HEADSTATE_LIVE_BRANCH to run this"
                );
                return;
            }
        };
        let a = assess(&repo, &wt, &branch);
        println!("--- ASSESSMENT ---\n{a:#?}");
        println!("--- PROMPT ---\n{}", a.prompt());
        println!("--- COMMAND ---\n{}", a.command("claude"));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn base() -> Assessment {
        Assessment {
            path: "/code/proj-feature".into(),
            branch: "feature/retry".into(),
            commits_ahead: Some(4),
            files_changed: Some(11),
            insertions: Some(240),
            deletions: Some(18),
            last_activity: Some("3 weeks ago".into()),
            has_upstream: true,
            subjects: vec!["add retry to the client".into()],
            subjects_elided: 0,
            uncommitted: 0,
            base: "origin/main".into(),
            fetched_at: Some("2026-09-11T08:00:00Z".into()),
        }
    }

    /// The most important fact in the struct: these commits exist nowhere
    /// else. It must be stated, and only when true.
    #[test]
    fn never_pushed_is_called_out_explicitly() {
        let p = Assessment {
            has_upstream: false,
            ..base()
        }
        .prompt();
        assert!(p.contains("NOT PUSHED"), "{p}");
        assert!(p.contains("only on this machine"), "{p}");

        assert!(!base().prompt().contains("NOT PUSHED"));
    }

    /// Uncommitted work is not in the diff against the default branch, so
    /// an agent told only about commits would assess an incomplete
    /// picture.
    #[test]
    fn uncommitted_files_are_mentioned_only_when_present() {
        let dirty = Assessment {
            uncommitted: 3,
            ..base()
        }
        .prompt();
        assert!(dirty.contains("3 uncommitted files"), "{dirty}");
        assert!(!base().prompt().contains("uncommitted"));
    }

    /// A branch with 200 commits must not produce a prompt nobody reads.
    #[test]
    fn the_commit_list_is_capped_and_says_what_it_elided() {
        let p = Assessment {
            subjects: (0..MAX_SUBJECTS).map(|i| format!("commit {i}")).collect(),
            subjects_elided: 190,
            ..base()
        }
        .prompt();
        assert_eq!(p.matches("  - commit").count(), MAX_SUBJECTS);
        assert!(p.contains("and 190 more"), "{p}");
    }

    /// A fact that could not be gathered is omitted, never guessed. "0
    /// commits ahead" when the check failed sends an agent looking for
    /// work that is there.
    #[test]
    fn missing_facts_are_omitted_rather_than_defaulted() {
        let p = Assessment {
            commits_ahead: None,
            files_changed: None,
            last_activity: None,
            ..base()
        }
        .prompt();
        assert!(!p.contains("0 commits ahead"), "{p}");
        assert!(!p.contains("files changed"), "{p}");
        // The questions still get asked -- an agent with fewer facts is
        // still more use than no prompt.
        assert!(p.contains("Recommend one"));
    }

    /// Print the prompt for a human to read as prose. Run manually:
    /// `cargo test --lib show_the_prompt -- --ignored --nocapture`
    ///
    /// `#[ignore]`d and assertion-free on purpose. It is a review tool,
    /// not a check: the tests around it pin the load-bearing phrases, but
    /// whether the whole thing READS as instructions an agent will follow
    /// is a judgment no assertion makes. A snapshot of the full text
    /// would fail on every wording change without telling anyone whether
    /// the new wording is better, so there is none.
    #[test]
    #[ignore]
    fn show_the_prompt() {
        println!("--- FRESH ---\n{}", base().prompt());
        println!(
            "--- NEVER FETCHED ---\n{}",
            Assessment {
                fetched_at: None,
                has_upstream: false,
                ..base()
            }
            .prompt()
        );
    }

    /// #815's core fix: the agent is told to refresh `origin` before
    /// believing any number in the prompt.
    ///
    /// Nothing on this path fetches -- deliberately, see the module
    /// header -- so the agent's own refresh is the only thing that makes
    /// the comparison current. Without this instruction an agent
    /// assessed against refs that were days old, did the work, and found
    /// it already upstream at PR time.
    #[test]
    fn the_prompt_tells_the_agent_to_refresh_the_remote_first() {
        let p = base().prompt();
        assert!(p.contains("git fetch origin"), "{p}");
        // Ordered BEFORE the questions. A refresh mentioned after them is
        // read after the agent has already believed the counts.
        let fetch = p.find("git fetch origin").expect("a fetch instruction");
        let questions = p.find("Then please:").expect("the question block");
        assert!(fetch < questions, "the refresh must come first:\n{p}");
    }

    /// The base ref is named, not described.
    ///
    /// "the default branch" answers to three different refs -- the local
    /// `main`, the merge-base the worktree was cut from, and
    /// `origin/main` -- and only the last is what the counts were
    /// measured against. An agent that resolves the phrase differently
    /// gets different numbers and cannot tell that it has.
    #[test]
    fn the_prompt_names_the_base_ref_rather_than_describing_it() {
        let p = base().prompt();
        assert!(p.contains("origin/main"), "{p}");
        assert!(
            !p.contains("the default branch"),
            "the vague phrase is what #815 was about:\n{p}"
        );
        // The count line and question 2 both carry the real ref.
        assert!(p.contains("4 commits ahead of origin/main"), "{p}");
        assert!(p.contains("merged into origin/main"), "{p}");
    }

    /// A purely local repository has no remote-tracking ref, so
    /// `default_branch` falls back to the bare branch name. The prompt
    /// must quote THAT, not an `origin/` ref that does not exist.
    #[test]
    fn a_local_only_repository_gets_its_own_base_ref_quoted() {
        let p = Assessment {
            base: "main".into(),
            ..base()
        }
        .prompt();
        assert!(p.contains("4 commits ahead of main"), "{p}");
        assert!(!p.contains("origin/main"), "{p}");
    }

    /// STOP, not "mention it in the report".
    ///
    /// A caveat answered in a report written after the work is done is
    /// the exact failure #815 describes. The instruction only saves
    /// anything if it halts the agent before it starts.
    #[test]
    fn the_prompt_stops_the_agent_when_the_work_is_already_upstream() {
        let p = base().prompt();
        assert!(p.contains("ALREADY on"), "{p}");
        assert!(p.contains("STOP"), "{p}");
        assert!(p.contains("do no further work"), "{p}");
        // The ways a branch lands without its own commits surviving --
        // a squash-merged branch is not "merged" by `rev-list`.
        assert!(p.contains("cherry-picked"), "{p}");
    }

    /// The prompt says how stale its own numbers are.
    ///
    /// The page has carried this caveat since #702 and the agent never
    /// saw it, so the agent had no way to know its inputs were
    /// as-of-a-fetch rather than live (#815).
    #[test]
    fn the_prompt_dates_the_numbers_it_quotes() {
        let p = base().prompt();
        assert!(p.contains("as of a fetch at 2026-09-11T08:00:00Z"), "{p}");
    }

    /// Never fetched is the STALEST state, not the freshest -- the same
    /// rule `refAge` follows on the page. Saying nothing here would
    /// silence the caveat in the case that most needs it.
    #[test]
    fn never_fetched_is_stated_as_the_worst_case() {
        let p = Assessment {
            fetched_at: None,
            ..base()
        }
        .prompt();
        assert!(p.contains("NEVER been fetched"), "{p}");
        assert!(!p.contains("as of a fetch at"), "{p}");
    }

    /// The agent is told not to write. A button labelled "assess" that
    /// pushes branches is a different feature.
    #[test]
    fn the_prompt_forbids_pushing_and_opening_prs() {
        let p = base().prompt();
        assert!(p.contains("Do not push"), "{p}");
        assert!(p.contains("open a pull request"), "{p}");
        assert!(p.contains("let me decide"), "{p}");
    }

    /// Single quoting is what keeps a branch name from executing. Checked
    /// against a real shell separately; this pins the encoding.
    #[test]
    fn shell_metacharacters_are_neutralised() {
        for hostile in ["$HOME", "`whoami`", "x'; echo PWNED; '", "semi;colon"] {
            let q = shell_quote(hostile);
            assert!(q.starts_with('\'') && q.ends_with('\''), "{q}");
            // Every embedded quote is closed-escaped-reopened, so the
            // string cannot terminate early and start a new command.
            let inner = &q[1..q.len() - 1];
            assert!(
                !inner.contains('\'') || inner.contains(r"'\''"),
                "unescaped quote in {q}"
            );
        }
    }

    /// A path with a space must survive the `cd`.
    #[test]
    fn a_path_with_a_space_stays_one_argument() {
        let cmd = Assessment {
            path: "/code/my project".into(),
            ..base()
        }
        .command("claude");
        assert!(cmd.starts_with("cd '/code/my project' &&"), "{cmd}");
    }

    /// shortstat omits a clause entirely when it is zero, so the numbers
    /// must be read independently rather than positionally.
    #[test]
    fn shortstat_parses_partial_lines() {
        assert_eq!(
            parse_shortstat(" 2 files changed, 8 insertions(+)"),
            (Some(2), Some(8), None)
        );
        assert_eq!(
            parse_shortstat(" 1 file changed, 3 deletions(-)"),
            (Some(1), None, Some(3))
        );
        assert_eq!(
            parse_shortstat(" 11 files changed, 240 insertions(+), 18 deletions(-)"),
            (Some(11), Some(240), Some(18))
        );
        assert_eq!(parse_shortstat(""), (None, None, None));
    }

    /// The base ref is used as git gives it, not re-prefixed (#757).
    ///
    /// `default_branch` used to return the bare short name, so this
    /// module built `origin/{}` itself. Now that it returns the
    /// remote-tracking ref, that same line would ask git for
    /// `origin/origin/main` -- and every field here comes from a git
    /// call that would simply fail, so the assessment would come back
    /// silently EMPTY rather than wrong-looking: no commit count, no
    /// diffstat, no subjects. An agent handed that prompt is told the
    /// branch did nothing.
    ///
    /// Nothing else covers this. `live_assess_a_real_worktree` is
    /// `#[ignore]`d and every other test here builds an `Assessment`
    /// literal, so a real repository is the only way to catch it.
    #[test]
    fn the_assessment_compares_against_the_remote_ref_without_doubling_it() {
        let ident = [
            ("GIT_AUTHOR_NAME", "octocat"),
            ("GIT_COMMITTER_NAME", "octocat"),
            ("GIT_AUTHOR_EMAIL", "octocat@invalid"),
            ("GIT_COMMITTER_EMAIL", "octocat@invalid"),
        ];
        let run_in = |dir: &Path, args: &[&str]| {
            let out = std::process::Command::new("git")
                .arg("-C")
                .arg(dir)
                .args(args)
                .envs(ident)
                .output()
                .unwrap();
            assert!(
                out.status.success(),
                "git {args:?}: {}",
                String::from_utf8_lossy(&out.stderr)
            );
        };

        let tmp = tempfile::TempDir::new().unwrap();
        let remote = tmp.path().join("remote.git");
        std::fs::create_dir_all(&remote).unwrap();
        run_in(&remote, &["init", "-q", "--bare", "-b", "main"]);

        let repo = tmp.path().join("proj");
        std::fs::create_dir_all(&repo).unwrap();
        run_in(&repo, &["init", "-q", "-b", "main"]);
        std::fs::write(repo.join("base.txt"), "base\n").unwrap();
        run_in(&repo, &["add", "-A"]);
        run_in(&repo, &["commit", "-q", "-m", "base"]);
        run_in(
            &repo,
            &["remote", "add", "origin", remote.to_str().unwrap()],
        );
        run_in(&repo, &["push", "-q", "-u", "origin", "main"]);
        run_in(&repo, &["remote", "set-head", "origin", "-a"]);

        let wt = tmp.path().join("proj-feature");
        run_in(
            &repo,
            &[
                "worktree",
                "add",
                "-q",
                "-b",
                "feature",
                wt.to_str().unwrap(),
                "main",
            ],
        );
        std::fs::write(wt.join("feature.txt"), "one\ntwo\n").unwrap();
        run_in(&wt, &["add", "-A"]);
        run_in(&wt, &["commit", "-q", "-m", "add the feature"]);

        let a = assess(repo.to_str().unwrap(), wt.to_str().unwrap(), "feature");
        assert_eq!(
            a.commits_ahead,
            Some(1),
            "an unresolvable base makes every count absent, not zero"
        );
        assert_eq!(a.files_changed, Some(1));
        assert_eq!(a.insertions, Some(2));
        assert_eq!(a.subjects, vec!["add the feature".to_string()]);

        // The ref the counts came from, carried by name so the prompt can
        // quote it (#815). Same value the comparison above used -- if
        // these two ever diverge, the prompt tells an agent to reproduce
        // a number against a ref that did not produce it.
        assert_eq!(a.base, "origin/main");
        assert!(
            a.prompt().contains("1 commit ahead of origin/main"),
            "{}",
            a.prompt()
        );

        // This fixture pushes and never fetches, so `FETCH_HEAD` does not
        // exist -- which the prompt must state as the worst case rather
        // than pass over in silence.
        assert_eq!(a.fetched_at, None);
        assert!(a.prompt().contains("NEVER been fetched"), "{}", a.prompt());
    }
}
