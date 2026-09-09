/// TypeScript mirrors of the Rust model in `src-tauri/src/github/model.rs`.
/// Field names and enum values are wire-format, not TS convention: serde
/// renames `CiState`/`MergeState` to lowercase and `ReviewState` to
/// snake_case, and every struct field is already snake_case. Do not
/// "clean up" the casing here -- it must match what `invoke()` actually
/// receives, byte for byte.

export type CiState = "success" | "failure" | "pending" | "none";
export type MergeState = "mergeable" | "conflicted" | "checking";
export type ReviewState = "approved" | "changes_requested" | "review_required" | "none";

export interface Label {
  name: string;
  color: string;
}

export interface PullRequest {
  /// GraphQL node ID, so a row can act without opening the detail view.
  id: string;
  number: number;
  title: string;
  url: string;
  repo: string;
  author: string;
  is_draft: boolean;
  /// The branch being merged, and the branch it merges into.
  head_ref: string;
  /// The head commit the row was rendered from, so an "update branch"
  /// click can tell GitHub which commit the user was looking at.
  head_oid: string;
  /// The head branch's Ref node id, for deleting it after merge. `null`
  /// once the branch is gone -- which is how the UI tells "already
  /// cleaned up" from "still there".
  head_ref_id: string | null;
  base_ref: string;
  created_at: string;
  updated_at: string;
  ci: CiState;
  merge: MergeState;
  /// GitHub's own merge-readiness summary.
  ///
  /// Richer than `merge`, which only distinguishes conflicts. `clean` is
  /// what makes a merge button honest: any other value means GitHub would
  /// reject or block the merge. Inlined rather than exported as a named
  /// type, since nothing imports the name.
  merge_status:
    | "clean"
    | "dirty"
    | "blocked"
    | "unstable"
    | "behind"
    | "draft"
    | "unknown";
  review: ReviewState;
  in_merge_queue: boolean;
  labels: Label[];
  comment_count: number;
  /// Review conversations still open on the current code. Resolved and
  /// outdated threads are excluded.
  unresolved_threads: number;
  /// Logins whose review is still outstanding.
  ///
  /// Empty is ORDINARY: repositories that assign reviewers through a
  /// bot return nothing here, and so does a solo account.
  requested_reviewers: string[];
  /// Assignees, used as a fallback when no reviewer was requested.
  assignees: string[];
  /// Who has already reviewed, and what they said.
  latest_reviews: { author: string; state: string }[];
}

/// `merged_week`/`merged_month` are real. The other five derived fields
/// always come back zero from the Rust layer today -- Task 13 derives them
/// client-side from the PR list. Typed here so callers get the shape right;
/// do not rely on their values.
export interface Stats {
  merged_week: number;
  merged_month: number;
  in_merge_queue: number;
  needs_attention: number;
  awaiting_review: number;
  ready_to_queue: number;
  blocked_by_comments: number;
}

/// One day of PR activity. `date` is `YYYY-MM-DD` in UTC, matching the
/// GitHub search qualifiers the counts come from.
export interface HistoryPoint {
  date: string;
  opened: number;
  merged: number;
}

/// The daily series plus the period comparisons that drive the delta cards.
///
/// Every period window ENDS YESTERDAY: today is still accumulating, and
/// comparing a partial day against complete periods drags every delta
/// downward. `points` still includes today, because the chart's shape is
/// informative even when the last bar is short.
export interface History {
  points: HistoryPoint[];
  week_current: number;
  week_previous: number;
  opened_week_current: number;
  opened_week_previous: number;
  month_current: number;
  month_previous: number;
}

/// The period comparisons alone. Fetched separately from the daily series
/// so the delta cards can render while the chart is still loading.
export interface Periods {
  week_current: number;
  week_previous: number;
  opened_week_current: number;
  opened_week_previous: number;
  month_current: number;
  month_previous: number;
}

export interface RepoCount {
  repo: string;
  merged: number;
}

/// Aggregates over a SAMPLE of recently merged PRs, not a lifetime census
/// -- `sample_size` is how many were actually examined, and the UI labels
/// the figures with it. `cycle_time_hours` is sorted ascending so
/// `percentile()` can index it directly.
/// One merged PR, enough to name and open it.
export interface MergedPr {
  number: number;
  title: string;
  url: string;
  repo: string;
  cycle_time_hours: number;
  size: number;
}

export interface MergedDetail {
  cycle_time_hours: number[];
  /// additions+deletions per PR, sorted ascending for percentile lookup.
  pr_sizes: number[];
  additions: number;
  deletions: number;
  changed_files: number;
  review_count: number;
  comment_count: number;
  sample_size: number;
  repo_counts: RepoCount[];
  slowest: MergedPr[];
  largest: MergedPr[];
}

/// Median cycle time this week against last.
///
/// `sampled` is true when either window held more merges than GitHub
/// returns in one page (100), meaning the medians describe a sample of
/// that week rather than all of it.
export interface CycleTrend {
  current_hours: number;
  previous_hours: number;
  current_count: number;
  previous_count: number;
  sampled: boolean;
}

/// Why a worktree can or cannot be removed.
///
/// An enum rather than a boolean because the UI has to explain itself:
/// "3 uncommitted files" is actionable where a greyed-out button is not.
/// `never_pushed` is the dangerous one -- 52 of 295 worktrees on this
/// machine have no upstream, so their commits exist nowhere else.
export type Safety =
  | { kind: "safe" }
  | { kind: "main_checkout" }
  | { kind: "dirty"; detail: number }
  | { kind: "unpushed"; detail: number }
  | { kind: "never_pushed" }
  /// The branch was created and never committed to -- a scratch
  /// worktree. Distinct from `never_pushed`, which claims commits exist
  /// only here: for a branch with none, that claim is false, and the
  /// row said it beside "0 commits ahead".
  | { kind: "empty" }
  | { kind: "unmerged" }
  /// The repository that owned this worktree is gone, so nothing about
  /// the checkout can be classified -- there is no git to run in it.
  | { kind: "orphaned" }
  /// Listed, but not yet classified. Distinct from `unknown`, which
  /// means the check ran and could not decide.
  | { kind: "pending" }
  | { kind: "unknown"; detail: string };

/// How a checkout stands against its tracked upstream, as of the last
/// fetch. Never live -- the scan reads refs on disk and does not fetch.
export type Upstream =
  | { kind: "current" }
  | { kind: "ahead"; n: number }
  | { kind: "behind"; n: number }
  | { kind: "diverged"; n: [number, number] }
  | { kind: "untracked" }
  | { kind: "detached" }
  | { kind: "unknown"; n: string };

export interface Worktree {
  path: string;
  branch: string;
  head: string;
  size_bytes: number | null;
  safety: Safety;
  is_main: boolean;
  /// `YYYY-MM-DD` when this branch landed in the default branch, when it
  /// can be determined. The date the work REACHED the default branch, not
  /// the branch tip's own commit date -- those diverge for a branch
  /// written weeks before it merged.
  merged_at: string | null;
  /// How this checkout stands against its upstream, for every row.
  upstream: Upstream | null;
  /// RFC 3339 timestamp of the branch tip's own commit. Not `merged_at`,
  /// which is when the work reached the default branch.
  last_commit: string | null;
}

export interface WorktreeRepo {
  /// `owner/repo` from the git REMOTE, not the directory name -- this
  /// app's own directory is `ghstat` while its repository is
  /// `pktstorm/headstate`. `null` when there is no remote to ask.
  identity: string | null;
  name: string;
  path: string;
  worktrees: Worktree[];
  /// When this repository's remote refs were last fetched, RFC 3339, or
  /// null if never fetched or unreadable.
  ///
  /// Every merge and upstream verdict below is computed against refs
  /// already on disk -- the scan never goes to the network on purpose.
  /// This is what lets the view say how old those answers are (#702).
  ///
  /// Optional in the TYPE so a fixture need not enumerate it, and
  /// `undefined` reads the same as `null` at every use: both mean the
  /// age is unknown, which is what the UI must say. The Rust side
  /// always sends the key.
  fetched_at?: string | null;
}

/// Everything the detail view renders.
///
/// Separate from `PullRequest`, which is a list row fetched 100 at a time
/// on a poll loop -- carrying a body and comments there would make every
/// tick haul data almost no row needs.
/// One review conversation on a pull request.
export interface ReviewThread {
  /// The thread's node id, which the resolve and reply commands take --
  /// NOT the pull request's id.
  id: string;
  is_resolved: boolean;
  /// Whether the anchored line still exists after a force-push.
  ///
  /// Not the same question as resolved: an outdated thread can still hold
  /// an unanswered question, so the UI must never present "the code moved"
  /// as "this was dealt with".
  is_outdated: boolean;
  path: string;
  /// Null once the anchor is gone, which is when `is_outdated` is true.
  /// Render the path alone rather than `file.ts:null`.
  line: number | null;
  /// What THIS viewer may do, per thread. Separate permissions because
  /// GitHub grants them separately; a button shown without its permission
  /// fails with a 403 on click.
  viewer_can_reply: boolean;
  viewer_can_resolve: boolean;
  viewer_can_unresolve: boolean;
  comments: { author: string; created_at: string; body: string }[];
  /// The true total, which can exceed `comments.length` -- the query
  /// pages thread comments at 10.
  comment_count: number;
}

export interface PrDetail {
  /// GraphQL node ID. Every mutation takes this rather than a number, so
  /// a write can only follow a read of the thing being written.
  id: string;
  number: number;
  title: string;
  url: string;
  state: string;
  is_draft: boolean;
  body: string;
  author: string;
  repo: string;
  head_ref: string;
  /// The head commit the row was rendered from, so an "update branch"
  /// click can tell GitHub which commit the user was looking at.
  head_oid: string;
  head_ref_id: string | null;
  base_ref: string;
  merge_status: string;
  review: string;
  /// Every reviewer's latest review state, keyed by login.
  ///
  /// A different question from `review`, which is the pull request's
  /// AGGREGATE decision: it reads "changes_requested" when someone else
  /// blocked it. Matching the viewer's login against this is the only
  /// way to answer "did MY approval land".
  latest_reviews: { author: string; state: string }[];
  /// Whether this pull request's base branch uses a merge queue.
  ///
  /// Chooses between Merge and Add to merge queue, so the user is not
  /// asked to pick between two buttons only one of which can work.
  merge_queue_enabled: boolean;
  /// Whether it is currently queued (and not rejected by the queue).
  in_merge_queue: boolean;
  additions: number;
  deletions: number;
  changed_files: number;
  unresolved_threads: number;
  comment_count: number;
  comments: { author: string; created_at: string; body: string }[];
  /// The review conversations -- inline threads anchored to a file and
  /// line. A DIFFERENT object from `comments` above, which are flat
  /// top-level comments: only threads can be resolved, so merging the two
  /// into one list would imply a Resolve button on comments that have no
  /// such concept.
  review_threads: ReviewThread[];
  /// `state` is `success`, `failure`, `pending`, `skipped`, or a raw
  /// GitHub value when unmodelled -- never coerced to success. Inlined
  /// rather than exported types, since nothing imports the names.
  checks: {
    name: string;
    state: string;
    url: string;
    /// The Actions workflow run, for re-running failed jobs. Null for a
    /// plain commit status or a non-Actions check -- neither can be
    /// re-run, so the button is offered only where this exists.
    run_id: number | null;
  }[];
}

/// How an image's provenance was established. A recorded fact and a
/// resolved guess should not look identical in the UI.
type OriginSource = "build_history" | "tag_resolution";

interface DockerOrigin {
  repo_path: string;
  /// The build context, which for a worktree build IS the worktree.
  context: string | null;
  commit: string;
  subject: string;
  /// The branch landed, so nothing will ever want this image again.
  merged: boolean;
  source: OriginSource;
}

export interface DockerImage {
  id: string;
  repository: string;
  /// Every tag pointing at this ID -- `latest` and a SHA are one image.
  tags: string[];
  created: string;
  size_bytes: number;
  origin: DockerOrigin | null;
  /// `null` means we could not ask -- NOT "nothing is using it". An
  /// unknown answer renders as not-removable.
  in_use: boolean | null;
  superseded: boolean;
  /// Another image shares this repository, newer or older.
  ///
  /// Separates "the newest of several" from "the only one there is" --
  /// with one image per repository nothing can ever be superseded, so
  /// `current` appeared on every row and discriminated nothing.
  has_siblings: boolean;
}

export interface DockerDiskUsage {
  images_bytes: number;
  images_reclaimable_bytes: number;
  build_cache_bytes: number;
  volumes_bytes: number;
  volumes_reclaimable_bytes: number;
}

/// Docker is frequently OFF, unlike git. "We could not ask" is not "the
/// answer is zero".
export type DockerState =
  | { kind: "running" }
  | { kind: "not_running" }
  | { kind: "permission_denied" }
  | { kind: "not_installed" }
  | { kind: "unknown"; detail: string };

export interface DanglingVolume {
  name: string;
  size_bytes: number;
}

export interface ImageRemovalOutcome {
  id: string;
  error: string | null;
}

export interface DockerBuild {
  reference: string;
  name: string;
  status: string;
  started: string;
  duration_secs: number;
  total_steps: number;
  cached_steps: number;
  /// Resolved on demand: `inspect` is a subprocess per build.
  context: string | null;
  revision: string | null;
}

/// What the app already knows about a worktree's unmerged work.
///
/// Mirrors the Rust `Assessment`. Every field was already computed for
/// the Claude Code handoff and then discarded except the shell string.
export interface Assessment {
  path: string;
  branch: string;
  commits_ahead: number | null;
  files_changed: number | null;
  insertions: number | null;
  deletions: number | null;
  /// Relative, as git prints it: "3 weeks ago".
  last_activity: string | null;
  /// Never pushed means these commits exist only on this machine.
  has_upstream: boolean;
  subjects: string[];
  subjects_elided: number;
}

/// What kind of build output a directory holds.
///
/// Mirrors `ArtifactKind` in Rust. The membership rule is that a
/// documented command rebuilds it -- which is what makes removal cost a
/// rebuild rather than work, and why this is a closed set rather than a
/// user-supplied pattern.
export type ArtifactKind =
  | "cargo_target"
  | "node_modules"
  | "terraform"
  | "dotnet_build"
  | "build_output";

/// One directory of regenerable build output.
export interface Artifact {
  /// Absolute path. Removal takes this, never a name matched by pattern.
  path: string;
  kind: ArtifactKind;
  /// The checkout it belongs to, for grouping.
  repo_path: string;
  /// Bytes on disk, or null until measured.
  ///
  /// Discovery and sizing differ by three orders of magnitude (measured:
  /// ~1.5s to find 178 directories, ~56s to size them), so the list
  /// renders before this is known. Null rather than 0: "not measured
  /// yet" and "empty" are different facts, and showing 0 B for the
  /// former is a lie the user would act on.
  size_bytes: number | null;
  /// Seconds since anything under it was written, or null if unknown.
  ///
  /// A running build does not make git dirty -- build output is
  /// gitignored -- so this is the only signal that a directory is in
  /// active use.
  modified_secs_ago: number | null;
}

/// The outcome of removing one artifact directory.
///
/// Per-directory rather than one verdict for the batch: a directory that
/// went active since the scan is refused while the rest succeed.
export interface ArtifactRemoval {
  path: string;
  /// Null on success. Shown verbatim -- it names WHY, and "could not
  /// remove" alone is not something a user can act on.
  error: string | null;
}

/// Why a Poetry virtualenv is reclaimable.
export type VenvState = "orphaned" | "stale" | "live";

/// One Poetry virtualenv.
export interface Venv {
  path: string;
  /// The project name Poetry encoded, e.g. `hello-world-delivery`.
  project: string;
  state: VenvState;
  /// The directory that produced it. Null for an orphan -- that IS the
  /// finding, not missing data.
  source: string | null;
  size_bytes: number | null;
  /// Seconds since the newest file INSIDE was written. Poetry touches a
  /// venv's root without writing inside, so its own mtime reports a
  /// year-old venv as days old.
  idle_secs: number | null;
}

export interface VenvRemoval {
  path: string;
  error: string | null;
}

/// One thing the automatic cleanup pass considered.
export interface LedgerEntry {
  at: string;
  /// `artifact` or `venv`.
  kind: string;
  target: string;
  /// An artifact's rebuild command, or a virtualenv's project.
  detail: string | null;
  bytes: number | null;
  /// `proposed`, `removed`, `refused`, or `skipped`.
  action: string;
  error: string | null;
}

/// Preferences for the automatic pass.
///
/// `mode` carries a `remove` variant so the stored shape does not change
/// in Phase 2, but this build refuses to store it: a setting that can be
/// turned on and does nothing is worse than one that does not exist.
export interface CleanupPrefs {
  enabled: boolean;
  mode: "preview" | "remove";
  artifacts: boolean;
  venvs: boolean;
  /// Whether an unattended pass may propose STALE virtualenvs, not just
  /// orphans. An orphan is a fact; stale is a threshold about a project
  /// that still exists, and that is what needs the opt-in here.
  venvs_stale: boolean;
  /// Merged branches. Parent of the two below.
  branches: boolean;
  /// Merged by ancestry — a graph fact, the strongest claim available.
  branches_ancestor: boolean;
  /// Merged by squash, found by comparing patch-ids. A CONTENT
  /// comparison rather than a graph one, so it gets its own opt-in for
  /// the same reason `venvs_stale` does — and it is the common case
  /// (489 of 536 on a real repository), so enabling it is not a small
  /// decision.
  branches_squash: boolean;
  worktrees: boolean;
  /// Merged, clean, and pushed — nothing is lost by removing one.
  worktrees_safe: boolean;
  docker: boolean;
  /// Untagged and referenced by nothing.
  docker_dangling: boolean;
  max_per_run: number;
}

export type Ecosystem =
  | "npm"
  | "yarn"
  | "poetry"
  | "uv"
  | "dotnet"
  | "cocoapods"
  | "terraform"
  | "swift"
  | "cargo";

/// How large a version jump is.
///
/// `unknown` is a real answer, not a fallback. Version schemes here are
/// not all semver -- .NET ships four parts, PEP 440 has epochs -- and a
/// version silently called major hides from a "minors only" filter while
/// one silently called minor is offered as safe.
export type Bump = "patch" | "minor" | "major" | "unknown";

export interface Outdated {
  name: string;
  current: string;
  latest: string;
  bump: Bump;
  ecosystem: Ecosystem;
  /// The manifest to edit, so an agent does not have to find it.
  manifest: string;
  /// Which PROJECT in the repository this row came from, relative to
  /// the repository root. Empty at the root.
  ///
  /// Attached by `PackagesPage` rather than sent by the backend: the
  /// grouping already knows it (`ProjectReport.label`) and flattening
  /// the groups for the wizard is what threw it away. An apply needs it
  /// -- in THIS repository every Rust row lives under `src-tauri` or
  /// `src-mobile` and there is no `Cargo.toml` at the root at all, so a
  /// request without it has nothing to edit.
  project?: string;
}

/// One package the user asked to update.
export interface UpdateRequest {
  name: string;
  version: string;
  ecosystem: Ecosystem;
  /// The project directory, relative to the repository root. Omitted or
  /// empty means the root itself, which is what every caller meant
  /// before this field existed.
  project?: string;
}

/// What happened to one requested update.
///
/// Per-package rather than one status for the run: updates apply in
/// sequence, and a failure in the third must not erase the report of the
/// two that worked.
interface UpdateOutcome {
  name: string;
  /// The version ASKED FOR.
  requested: string;
  /// Files git reports as changed. Empty means the command succeeded and
  /// changed nothing -- usually a manifest constraint pinning the
  /// package below the requested version, which is worth showing.
  changed_files: string[];
  /// The tool's own output, kept on success too: resolvers warn about
  /// peer conflicts while still succeeding.
  output: string;
  /// The constraint the manifest holds AFTERWARDS.
  ///
  /// Not the same as `requested`, and that is the point: npm rewrites a
  /// pinned `4.17.21` request into `^4.17.21`, a range rather than a
  /// pin. `null` when it could not be read, which is shown as unknown
  /// rather than assumed to match.
  resolved_constraint: string | null;
  /// Set when this package failed; the others still report.
  error: string | null;
}

/// The result of an update run.
export interface RunReport {
  /// Which ecosystems the run touched. Opening a pull request is only
  /// offered where the resolved constraint can be read back.
  ecosystems: Ecosystem[];
  /// The worktree holding the changes. Phase 1 does not push, so this
  /// path IS the deliverable.
  worktree: string;
  branch: string;
  results: UpdateOutcome[];
}

/// What one ecosystem reported for one repository.
///
/// `error` exists because "no updates" and "the check did not run" are
/// opposite answers, and rendering both as an empty list reports failure
/// as good news.
export interface EcosystemReport {
  ecosystem: Ecosystem;
  outdated: Outdated[];
  error: string | null;
}

export type UpdateFilter = "patch" | "minor" | "all";

/// One imported file in a CLAUDE.md tree.
export interface ImportNode {
  /// What the file wrote, verbatim.
  raw: string;
  /// Where it resolved to, when it did.
  path: string | null;
  bytes: number;
  tokens: number;
  /// Why this node is unusable, when it is. A broken or circular import
  /// is SHOWN rather than dropped -- omitting it makes the tree look
  /// complete when it is not.
  problem: string | null;
  children: ImportNode[];
}

/// One CLAUDE.md and the tree it pulls in.
export interface ClaudeFile {
  path: string;
  bytes: number;
  /// ESTIMATED tokens for this file alone. Characters divided by four,
  /// not a real tokeniser -- every label says so.
  tokens: number;
  /// Estimated tokens for this file plus everything it imports. The
  /// number that matters: a 2 KB file pulling in 40 KB of imports is the
  /// case this view exists to surface.
  total_tokens: number;
  imports: ImportNode[];
}

/// One project's worth of reports.
///
/// The unit the UI groups by. A repository can hold several -- a
/// frontend and a backend are separate manifests and often separate
/// ecosystems, so their updates are separate pieces of work.
export interface ProjectReport {
  /// Absolute path to the project directory.
  path: string;
  /// Relative to the repository root. Empty at the root itself.
  label: string;
  reports: EcosystemReport[];
}

/// Why a branch may or may not be deleted.
///
/// Reports the fact rather than a verdict, so the UI can say WHY a
/// branch is not deletable instead of only greying out a control.
export type Deletable =
  /// `squash` comes from comparing patch-ids -- a content comparison,
  /// not a graph one. Measured on a real repository, 489 of 536 merged
  /// branches were squashes, so it is the common case, not the exotic
  /// one, and the UI says which.
  | { kind: "merged"; how: "ancestor" | "squash" }
  | { kind: "defaultBranch" }
  | { kind: "checkedOut"; path: string }
  | { kind: "unmerged"; ahead: number }
  | { kind: "pending" }
  | { kind: "unknown"; reason: string };

export interface Branch {
  name: string;
  /// The three cases clean up differently, which is why this is one
  /// value rather than a pair of booleans: deleting a tracked pair is
  /// two operations against two different things.
  location: "local" | "remote" | "tracked";
  upstream: string | null;
  ahead: number;
  behind: number;
  /// ISO 8601, as git reports it.
  committed: string;
  author: string;
  tip: string;
  deletable: Deletable;
}

export interface DeleteOutcome {
  name: string;
  /// `null` on success; the reason otherwise.
  error: string | null;
}

/// One frame of a running branch scan, mirroring the Rust
/// `BranchScanFrame` in `src-tauri/src/commands.rs`.
///
/// Two shapes on one event name because it is one stream. `listed`
/// arrives once, with every row and — crucially — the TOTAL; then
/// `classified` frames carry verdicts as the eight classification
/// threads settle them.
///
/// The total is what separates a dead stream from a finished one. Fed
/// only verdicts, a page that stopped receiving them at 47 would look
/// exactly like a page that had received all of them; knowing 512 were
/// promised, it can say so (#657).
///
/// `repo` is on every frame and is load-bearing: the events are
/// app-global while a scan is per-repository, so a page that switched
/// repository mid-scan would otherwise fold the old repository's
/// verdicts into the new one's rows.
export type BranchScanFrame =
  | { kind: "listed"; repo: string; total: number; branches: Branch[] }
  | { kind: "classified"; repo: string; verdicts: [string, Deletable][] };

/// One frame of a running branch DELETION, mirroring the Rust
/// `BranchDeleteFrame` in `src-tauri/src/commands.rs`.
///
/// Two shapes because a deletion is two phases with different
/// meanings, and collapsing them into one counter is the bug this
/// exists to fix. The safety re-check is a full uncached scan at ~64ms
/// per branch, so on the 562-branch batch that was reported a single
/// counter would read 0/562 for MINUTES — indistinguishable from a
/// hang — before a single ref came off (#724).
///
/// `checking` therefore counts branches the re-check has classified,
/// and its total is every branch in the repository, since that is what
/// the gate scans. `deleting` counts the selected batch, and carries
/// `failed` on every frame so refusals are visible while the run is
/// still going rather than only in the toasts afterwards.
///
/// Counts only: no branch names, no paths. Unlike the scan's frames,
/// which are filling a list of names in, nothing here needs a join key.
export type BranchDeleteFrame =
  | { kind: "checking"; repo: string; done: number; total: number }
  | { kind: "deleting"; repo: string; done: number; total: number; failed: number };

/// One moment of the machine's health, mirroring the Rust
/// `health::Sample` in `src-tauri/src/health/mod.rs`.
///
/// # Absent is not zero
///
/// Every optional field here is `null` when the platform does not
/// expose it, never `0`. The UI must render those as "not measured":
/// a zero that means "we could not look" reads as a real reading, and
/// "0% CPU" and "we did not measure the CPU" are opposite claims. This
/// is the same rule the Rust side states in its module docs.
export interface HealthSample {
  /// RFC 3339, matching every other timestamp this app stores.
  sampled_at: string;
  /// 1, 5 and 15 minute load averages, or `null` on a platform with no
  /// equivalent (Windows).
  load: [number, number, number] | null;
  /// Whole-machine CPU use, 0-100.
  cpu_percent: number | null;
  /// Per-core, 0-100, in the platform's own core order. Empty rather
  /// than null when unavailable, matching the Rust `Vec`.
  cpu_per_core: number[];
  memory: HealthMemory;
  /// Every GPU the platform would describe, which on several is none.
  ///
  /// Empty is "nothing discoverable" -- a platform Headstate cannot
  /// read unprivileged (Windows, and Intel/NVIDIA on Linux), or a
  /// machine with no GPU. The UI draws NO PANEL for an empty list
  /// rather than a panel of zeroes: a 0% GPU is a claim about an idle
  /// GPU, and "we did not look" is the opposite claim.
  gpus: HealthGpu[];
  disks: HealthVolume[];
  battery: HealthBattery | null;
  /// `nominal` / `fair` / `serious` / `critical`.
  ///
  /// NOT a temperature. The SMC needs elevated privileges on macOS, so
  /// what is actually readable is the platform's thermal PRESSURE --
  /// a coarse label. The UI says so; see `SystemHealthPage`.
  thermal: string | null;
  networks: HealthInterface[];
  /// Seconds since boot.
  uptime_secs: number;
}

/// The four unexported types below match `DockerOrigin` and
/// `UpdateOutcome` above: nothing outside this file names them, since
/// every consumer reaches them through `HealthSample`. Exporting a name
/// no one imports is a name that has to be kept correct for no reader.
interface HealthMemory {
  total: number;
  used: number;
  /// What the OS believes is reclaimable, which is NOT `total - used`
  /// on any modern platform: cache counts as used and is available.
  available: number;
  swap_total: number;
  swap_used: number;
}

/// One GPU, mirroring the Rust `health::Gpu` in
/// `src-tauri/src/health/gpu.rs`.
///
/// Exported, unlike the other `HealthSample` helpers, for the same
/// reason as `FootprintProcess`: `SystemHealthPage` renders a component
/// per GPU and therefore has to name the type.
///
/// Every field but `name` is nullable, because the platforms disagree
/// about which they answer. macOS reports all of them; AMD on Linux
/// reports all of them; a machine that reports only utilization leaves
/// the memory pair null, and the UI renders that as "Not measured"
/// rather than as 0 bytes.
export interface HealthGpu {
  /// The adapter as the platform names it -- "Apple M2 Max", or the
  /// DRM card on Linux. Never a path.
  name: string;
  /// Whole-device utilization, 0-100.
  utilization_percent: number | null;
  memory_used: number | null;
  /// On a unified-memory machine this is the share currently allocated
  /// to the GPU, NOT a dedicated pool. See `unified_memory`.
  memory_total: number | null;
  /// True when the GPU shares the system's memory rather than having
  /// its own, which is every Apple Silicon Mac.
  ///
  /// The UI MUST say so where this is true. The Memory panel reports
  /// the same physical pool, so without that sentence the two panels
  /// look like they disagree about how much memory the machine has --
  /// and a reader would reasonably add the GPU's gigabytes to the
  /// system's and conclude the machine has more RAM than it does.
  unified_memory: boolean;
}

interface HealthVolume {
  mount: string;
  total: number;
  available: number;
  /// True for the volume the app itself lives on, which is the one a
  /// user filling their disk cares about first.
  is_root: boolean;
}

interface HealthBattery {
  percent: number;
  on_ac: boolean;
}

interface HealthInterface {
  name: string;
  /// Cumulative since boot, not since the last sample.
  rx_bytes: number;
  tx_bytes: number;
}

/// What Headstate itself is costing at one instant, mirroring the Rust
/// `health::Footprint` in `src-tauri/src/health/footprint.rs`.
///
/// # Absent is not zero, again
///
/// The same rule as `HealthSample`, and here it is easier to get wrong,
/// because every field that can be missing is missing in the ORDINARY
/// case rather than the exotic one. `git` is not running most of the
/// time; most machines have no Docker daemon up. So a UI that defaults
/// any of these to a zero does not merely mislead in an edge case -- it
/// tells almost every user, almost always, that a tool is running and
/// idle when it never started.
export interface Footprint {
  /// RFC 3339, stamped by the Rust side at the moment of the reading.
  sampled_at: string;
  /// The Tauri host process, or `null` if the platform would not report
  /// our own PID. Not expected anywhere this ships, but a fabricated
  /// zero for "we could not find ourselves" would read as an idle app.
  app: FootprintProcess | null;
  /// One entry per LIVE `git` / `gh` / `du` / `docker` process.
  ///
  /// Empty means none were running at that instant, which is the
  /// ordinary state between refreshes -- NOT a row of zeroes. Several
  /// entries may share a `name`: a worktree scan runs many `git` at
  /// once, and the Rust side deliberately does not collapse them,
  /// because that fan-out is the thing this panel exists to show.
  ///
  /// Already sorted biggest-first with ties broken by PID, so the
  /// caller neither has to sort nor should: the stable order is what
  /// stops the list reshuffling between five-second polls.
  children: FootprintProcess[];
  /// The Docker daemon, or `null` when Docker is not running -- which
  /// is the common answer, and precisely why it must not be a zero.
  docker_daemon: FootprintProcess | null;
  /// The biggest CPU consumers on the WHOLE machine, biggest first
  /// (#687).
  ///
  /// Not Headstate's -- the three fields above are ours. This is what
  /// the CPU detail page shows, because "CPU is at 80%" is a symptom
  /// and "these are the processes" is the answer.
  ///
  /// A bounded TOP N (eight), never the full list: 1400-odd rows is not
  /// an answer to "what is using my CPU", it is the same filtering
  /// problem handed back to the reader. `process_count` says how many
  /// there were, so a short list never has to be mistaken for the whole
  /// machine.
  ///
  /// OPTIONAL, and the optionality is load-bearing rather than
  /// defensive. This was challenged on review, checked against the
  /// version machinery, and the answer is that the machinery does not
  /// reach this case. Worth spelling out, because the two mechanisms
  /// that look like they cover it are real -- they just bite elsewhere:
  ///
  /// - **The version gate is on WRITES.** `connection.rs`'s `blocked()`
  ///   refuses a desktop below `PROTOCOL_VERSION`, but its one caller
  ///   (`companion.rs`) guards it with `matches!(class, Class::Write |
  ///   Class::Destructive)` and says why: reads go through whatever the
  ///   state, because the attempt is how the phone learns the desktop
  ///   is back. `system_footprint` is `Class::Read`.
  /// - **Cert pinning refuses protocol 1, not "older".** ML-DSA-65
  ///   certificates were the 1-to-2 change (#521), so a 5.0 desktop
  ///   fails the handshake. A protocol-2 desktop from before this
  ///   feature completes it normally.
  ///
  /// What remains is not a protocol mismatch at all. The companion
  /// ships on its own tag, independent of the desktop's
  /// (`docs/mobile-release-process.md`), and compatibility is the wire
  /// protocol's integer -- which adding fields to a response does not
  /// bump, because doing so is backward-compatible. So a phone carrying
  /// this feature paired with a desktop released before it is a
  /// protocol-2-to-protocol-2 pairing: allowed, unblocked, and missing
  /// these fields. Two release pipelines make that ordering ordinary.
  ///
  /// The absence is a different fact from an empty list -- "that desktop
  /// cannot tell us" versus "nothing is running", the latter impossible
  /// on a booted machine -- and the UI renders the two differently.
  /// `call<Footprint>` is an unchecked cast, so a required type here
  /// would have the compiler certify a guarantee the wire does not give.
  top_cpu?: FootprintProcess[];
  /// The same, by resident size. A SEPARATE list rather than `top_cpu`
  /// re-sorted: the process pinning a core is rarely the one holding
  /// 8 GB, and re-sorting one list by the other metric would show the
  /// top of a set that was chosen by the wrong measure.
  ///
  /// Optional for the same version-skew reason as `top_cpu`.
  top_memory?: FootprintProcess[];
  /// How many processes were running when the two lists were taken.
  ///
  /// So the UI can say what it is not showing. Eight of 1436 is a
  /// defensible answer; eight presented as everything is not.
  ///
  /// Optional for the same reason as the two lists. When it is absent
  /// the UI omits the "of N running" sentence rather than inventing a
  /// total -- a count that does not exist must not be rendered as one
  /// that does.
  process_count?: number;
}

/// One process in a `Footprint`.
///
/// Exported, unlike the four `HealthSample` helpers above, because
/// `SystemHealthPage` renders a row component that takes one of these
/// directly and therefore has to name the type.
export interface FootprintProcess {
  pid: number;
  /// The executable's own name as the OS reports it -- `git`, or
  /// `git.exe` on Windows. Never a full path.
  name: string;
  /// CPU use as a percentage of ONE core, so legitimately above 100 for
  /// a process using more than one, which `git` does. The UI must not
  /// clamp this the way it can clamp `HealthSample.cpu_percent`.
  cpu_percent: number;
  /// Resident set size in bytes: physical RAM held right now. Not
  /// virtual size, which on anything linking a webview is a large
  /// number that means nothing to a reader.
  memory: number;
}
