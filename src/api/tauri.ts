/// Typed wrappers around the Tauri command surface in
/// `src-tauri/src/commands.rs`. Every command returns `Result<T, String>` on
/// the Rust side, which Tauri surfaces as a *rejected* promise (not a
/// resolved `Err` value) when the Rust side returns `Err`. Callers that need
/// to distinguish "not authenticated" from a network failure should inspect
/// the rejection message; `AuthGate` covers the common case by gating
/// render on `get_auth_state` before anything else calls in.
///
/// Each wrapper goes through `transport.call` rather than Tauri's
/// `invoke` directly, so the same signatures can be served by a network
/// client on the mobile companion. On the desktop the transport IS
/// `invoke`; see `transport.ts`.

import { call } from "./transport";
import type {
  Artifact,
  Branch,
  DeleteOutcome,
  ClaudeFile,
  ProjectReport,
  UpdateRequest,
  UpdateFilter,
  CleanupPrefs,
  LedgerEntry,
  Venv,
  VenvRemoval,
  ArtifactRemoval,
  Assessment,
  CycleTrend,
  DanglingVolume,
  DockerBuild,
  DockerDiskUsage,
  DockerImage,
  DockerState,
  History,
  ImageRemovalOutcome,
  MergedDetail,
  Periods,
  PrDetail,
  PullRequest,
  Stats,
  Worktree,
  WorktreeRepo,
} from "../types/pr";

export interface AuthState {
  ok: boolean;
  message: string;
}

/// The cached snapshot. Never talks to GitHub. Returns `[]` both when
/// nothing has ever been polled and when auth failed at startup -- callers
/// must consult `getAuthState` to tell those apart.
export const getCached = () => call<PullRequest[]>("get_cached");

/// A user-initiated, out-of-band fetch. Does not persist to SQLite and does
/// not affect the poll loop's cadence.
export const refreshNow = () => call<PullRequest[]>("refresh_now");

/// Interface preferences. Mirrors the Rust `UiPrefs`.
export interface UiPrefs {
  hidden_views: string[];
  close_hides_to_tray: boolean;
  announce_updates: boolean;
  /// Whether to write the verbose `[diag]` timing log.
  ///
  /// Kept as a switch rather than removed after v3.5.3: the next
  /// "it is slow on my machine" report wants exactly this log, and
  /// asking someone to install a special build to produce it is much
  /// worse than a checkbox.
  diagnostic_logging: boolean;
  /// Days idle before a virtualenv counts as stale. 0 means the default.
  stale_venv_days: number;
}

export const getUiPrefs = () => call<UiPrefs>("get_ui_prefs");
export const setUiPrefs = (prefs: UiPrefs) => call<void>("set_ui_prefs", { prefs });

/// Whether the app starts at login.
///
/// Asked of the OS rather than stored: the user can turn it off in
/// System Settings, and a stored flag would then disagree with reality.
export const getAutostart = () => call<boolean>("get_autostart");
export const setAutostart = (enabled: boolean) =>
  call<void>("set_autostart", { enabled });
/// Whether phones can connect to this desktop right now.
///
/// The live state rather than the stored setting, for the same reason
/// as autostart: the two differ when the port could not be bound at
/// startup, and the checkbox should say what is true.
export const getRemoteEnabled = () => call<boolean>("get_remote_enabled");
export const setRemoteEnabled = (enabled: boolean) =>
  call<void>("set_remote_enabled", { enabled });
/// What the app already knows about a worktree's unmerged work.
///
/// Per row rather than per scan: several git calls each, so this is
/// asked only when a user actually opens a row.
export const assessWorktree = (repoPath: string, worktreePath: string, branch: string) =>
  call<Assessment>("assess_worktree", { repoPath, worktreePath, branch });

/// Which desktop notifications the user wants.
///
/// Mirrors the Rust `NotifyPrefs`. Absent on the Rust side means
/// everything on, matching what the app did before the setting existed.
export interface NotifyPrefs {
  enabled: boolean;
  ci_failed: boolean;
  conflicted: boolean;
  /// Notify when a pull request enters the "Ready for review" set: green
  /// checks, no blockers, and the user is a requested reviewer.
  ready_to_review: boolean;
}

export const getNotifyPrefs = () => call<NotifyPrefs>("get_notify_prefs");

export const setNotifyPrefs = (prefs: NotifyPrefs) =>
  call<void>("set_notify_prefs", { prefs });

/// Re-run the failed jobs of a pull request's CI.
///
/// Takes the workflow RUN id, not a check id: one call re-runs every
/// failed job in that run, and a per-check call could not restart a job
/// that never started because an earlier one failed.
export const rerunChecks = (repo: string, number: number, runId: number) =>
  call<void>("rerun_checks", { repo, number, runId });

/// The platform and architecture this build was compiled for.
///
/// From Rust compile-time constants rather than the webview, so it
/// cannot disagree with the binary the user is running.
export const buildTarget = () => call<[string, string]>("build_target");

/// How many pull requests await the user's review.
///
/// A count, not a list: the sidebar badge needs a number on every view,
/// and asking for the list costs 6 rate-limit points and ~4s against 1
/// and ~0.9s for this.
export const countReviewing = () => call<number>("count_reviewing");

/// The authenticated user's login.
///
/// Asked once and cached forever: it cannot change during a session.
export const getViewer = () => call<string>("get_viewer");

/// A review verdict. Mirrors the Rust `ReviewVerdict`; the strings must
/// match `parse_verdict` in commands.rs exactly, which rejects anything
/// else rather than guessing.
///
/// GitHub's schema also has DISMISS, deliberately unreachable here: it
/// dismisses someone else's review, which nothing in the UI asks for.
export type ReviewVerdictName = "approve" | "request_changes" | "comment";

/// Submit a review on a pull request.
///
/// The first write to a PR the user does not own. `body` may be empty
/// only for `approve`; the Rust side rejects the other two without it
/// rather than letting GitHub refuse after a round-trip.
export const reviewPr = (
  id: string,
  repo: string,
  number: number,
  verdict: ReviewVerdictName,
  body: string,
) => call<void>("review_pr", { id, repo, number, verdict, body });

/// Comment on a pull request without reviewing it.
export const commentOnPr = (id: string, repo: string, number: number, body: string) =>
  call<void>("comment_on_pr", { id, repo, number, body });

/// Resolve a review conversation. Takes the THREAD's id, not the PR's.
export const resolveThread = (threadId: string, repo: string, number: number) =>
  call<void>("resolve_thread", { threadId, repo, number });

/// Reopen a resolved conversation -- the undo for `resolveThread`.
export const unresolveThread = (threadId: string, repo: string, number: number) =>
  call<void>("unresolve_thread", { threadId, repo, number });

/// Reply inside a conversation, keeping the answer attached to the code
/// it is about rather than starting a new top-level comment.
export const replyToThread = (
  threadId: string,
  repo: string,
  number: number,
  body: string,
) => call<void>("reply_to_thread", { threadId, repo, number, body });

/// `Stats.merged_week`/`merged_month` are real; the other five fields
/// always come back zero today. Does not persist to SQLite.
export const getStats = () => call<Stats>("get_stats");

/// Repos and their worktrees, WITHOUT safety classification.
///
/// ~800ms for 37 repos and 295 worktrees; safe to block a view on.
export const listWorktrees = () => call<WorktreeRepo[]>("list_worktrees");

/// Classify one repo's worktrees. Four git calls each, ~16s across all
/// 295 -- so this is per repo, filling in as results arrive.
export const classifyWorktrees = (repoPath: string) =>
  call<Worktree[]>("classify_worktrees", { repoPath });

/// Apply an action to a pull request.
///
/// Rejects with GitHub's own message on refusal -- "base branch was
/// modified" is display-ready and more useful than a substitute.
export const actOnPr = (
  id: string,
  repo: string,
  number: number,
  action: PrActionName,
) => call<void>("act_on_pr", { id, repo, number, action });

/// One worktree's outcome in a bulk removal. `error` is null on success.
export interface RemovalOutcome {
  path: string;
  error: string | null;
}

/// Remove several worktrees, each safety-checked independently at delete
/// time. Resolves with an outcome per worktree rather than throwing on
/// the first refusal: partial failure is the normal case.
export const removeWorktrees = (repoPath: string, worktreePaths: string[]) =>
  call<RemovalOutcome[]>("remove_worktrees", { repoPath, worktreePaths });

/// The newest published release, or null when this build is current.
///
/// Distribution is dmg/exe/deb/AppImage, so no package manager carries
/// updates: a user on a version with a launch-blocking bug otherwise has
/// no way to learn a fix exists.
export const latestRelease = () => call<string | null>("latest_release");

/// --- Docker -------------------------------------------------------

export const dockerState = () => call<DockerState>("docker_state");
export const dockerBuilds = () => call<DockerBuild[]>("docker_builds");
export const dockerImages = () => call<DockerImage[]>("docker_images");
export const dockerDiskUsage = () => call<DockerDiskUsage>("docker_disk_usage");
export const dockerRemoveImages = (ids: string[]) =>
  call<ImageRemovalOutcome[]>("docker_remove_images", { ids });
export const dockerDanglingVolumes = () => call<DanglingVolume[]>("docker_dangling_volumes");
export const dockerRemoveVolume = (name: string) =>
  call<void>("docker_remove_volume", { name });
/// Returns bytes actually freed, read from the command's own output
/// rather than echoed from an estimate.
export const dockerPruneCache = (until?: string) =>
  call<number>("docker_prune_cache", { until });
export const dockerRunningContainers = () => call<string[]>("docker_running_containers");
export const dockerRestart = () => call<void>("docker_restart");
export const dockerStart = () => call<void>("docker_start");

/// Worktrees handed to Claude Code and still at the head they were
/// assessed at. A branch that has moved since is dropped: the assessment
/// described a different state.
export const assessedWorktrees = () => call<string[]>("assessed_worktrees");

/// Remove a worktree the safety gate refuses. Reached only from a
/// confirmation opened after reading an assessment of that worktree.
export const removeWorktreeForced = (repoPath: string, worktreePath: string) =>
  call<void>("remove_worktree_forced", { repoPath, worktreePath });

/// The clipboard payload for Claudify, plus whether Claude Code was
/// found. `claude_installed` is advisory: the command is returned either
/// way, since a user may paste it on another machine.
export interface ClaudifyCommand {
  command: string;
  claude_installed: boolean;
}

/// The shell command that hands a worktree to Claude Code.
///
/// Text for the clipboard, not a spawn. Spawning a terminal is not
/// portable -- macOS has no default-terminal concept, and on Linux
/// `gio open` on a shell script opens an editor -- and the clipboard
/// lands the user in their own shell, where `claude` resolves even
/// though a GUI app's PATH does not include it.
export const claudifyCommand = (repoPath: string, worktreePath: string, branch: string) =>
  call<ClaudifyCommand>("claudify_command", { repoPath, worktreePath, branch });

/// Merge a pull request when its checks pass, or cancel that.
///
/// Takes the head OID the row was rendered from: auto-merge fires later
/// and unattended, so without the guard a push after enabling would
/// merge a commit the user never saw.
export const setAutoMerge = (
  id: string,
  repo: string,
  number: number,
  expectedHead: string,
  enable: boolean,
) => call<void>("set_auto_merge", { id, repo, number, expectedHead, enable });

/// Delete a merged pull request's head branch.
///
/// `merged` is re-checked on the Rust side: deleting the head ref of an
/// OPEN pull request closes it off.
export const deleteHeadBranch = (
  refId: string,
  repo: string,
  number: number,
  branch: string,
  merged: boolean,
) => call<void>("delete_head_branch", { refId, repo, number, branch, merged });

/// Merge the base branch into a pull request's head -- GitHub's "Update
/// branch" button.
///
/// Separate from `actOnPr` because it needs `expectedHead`: GitHub
/// refuses if the branch moved since the row was rendered, so a stale
/// click reports an error rather than updating a commit the user never
/// saw. Pass the `head_oid` from the same row that showed the button.
export const updatePrBranch = (
  id: string,
  repo: string,
  number: number,
  expectedHead: string,
) => call<void>("update_pr_branch", { id, repo, number, expectedHead });
/// One pull request's outcome in a batch. `error` is null on success.
export interface BatchOutcome {
  repo: string;
  number: number;
  error: string | null;
}

/// Apply one action to several pull requests.
///
/// Returns an outcome per pull request rather than throwing on the first
/// rejection: partial failure is the normal case for a batch, and a
/// single verdict would hide the rejections.
export const actOnPrs = (
  prs: [string, string, number][],
  action: PrActionName,
) => call<BatchOutcome[]>("act_on_prs", { prs, action });

/// The actions the backend accepts. A union rather than `string`, so a
/// typo is a compile error instead of a runtime "unknown action".
export type PrActionName =
  | "merge"
  | "close"
  | "reopen"
  | "draft"
  | "ready"
  | "enqueue"
  | "dequeue";

/// Everything the detail view shows for one pull request. Cost 1.
export const getPrDetail = (repo: string, number: number) =>
  call<PrDetail>("get_pr_detail", { repo, number });

/// Disk sizes for one repo's worktrees, as `[path, bytes]` pairs.
///
/// A full tree walk -- ~13s for 147 worktrees -- so it is a separate
/// query from listing and classification, and arrives last.
export const sizeWorktrees = (repoPath: string) =>
  call<[string, number][]>("size_worktrees", { repoPath });

/// Remove a worktree. Rejects anything not provably safe; the gate is
/// re-evaluated on the Rust side rather than trusted from the last scan.
/// Fast-forward a checkout to its upstream. Refuses on a dirty tree and
/// fast-forwards only; returns git's own output or its own refusal.
export const pullCheckout = (path: string) => call<string>("pull_checkout", { path });

/// Delete an orphaned worktree directory.
///
/// Separate from `removeWorktree` because git cannot remove it -- the
/// repository that owned it is gone. The Rust side re-checks that the
/// path is still orphaned before deleting anything.
export const removeOrphan = (path: string) => call<void>("remove_orphan", { path });

export const removeWorktree = (repoPath: string, worktreePath: string) =>
  call<void>("remove_worktree", { repoPath, worktreePath });

/// Tell the poll loop whether the active view needs live PR data.
export const setViewNeedsGithub = (needs: boolean) =>
  call<void>("set_view_needs_github", { needs });

/// Directories scanned for git checkouts. Defaults to `~/code`.
export const getWorktreeDirs = () => call<string[]>("get_worktree_dirs");

/// Replace the scanned directories. Rejects paths that are not
/// directories, so a typo fails here rather than yielding an empty view.
export const setWorktreeDirs = (dirs: string[]) =>
  call<string[]>("set_worktree_dirs", { dirs });

/// The configured focused poll interval, in seconds.
export const getPollInterval = () => call<number>("get_poll_interval");

/// Set the poll interval. Returns the value actually applied, which may be
/// clamped -- the UI shows what the backend accepted, not what was asked.
export const setPollInterval = (secs: number) =>
  call<number>("set_poll_interval", { secs });

/// PRs awaiting the user's review. Rides along in the same GraphQL
/// document as the authored list, so it costs no extra rate limit.
export const getReviewing = () => call<PullRequest[]>("get_reviewing");
/// The last successful review list, straight from SQLite. Never talks
/// to GitHub.
export const getCachedReviewing = () => call<PullRequest[]>("get_cached_reviewing");

/// Median cycle time this week against last, in one request.
export const getCycleTrend = () => call<CycleTrend>("get_cycle_trend");

/// The period comparisons alone -- one small request (~1.6s) so the delta
/// cards paint without waiting on the daily series.
export const getPeriods = () => call<Periods>("get_periods");

/// The daily opened/merged series plus period comparisons. Fetched as
/// concurrent chunks on the Rust side. `days` is clamped to 1..=90.
export const getHistory = (days: number) => call<History>("get_history", { days });

/// Aggregates over the most recent 100 merged PRs. A separate command from
/// `getHistory` on purpose: it is the more expensive of the two and only
/// the insight row needs it, so a failure here must not blank the chart.
export const getMergedDetail = () => call<MergedDetail>("get_merged_detail");

/// Computed once at startup from the `gh` CLI token. `ok: false` means the
/// user needs to run `gh auth login`; `message` is ready-to-display prose.
export const getAuthState = () => call<AuthState>("get_auth_state");

/// Regenerable build output under the configured scan roots.
///
/// Discovery only: every `size_bytes` comes back null. See
/// `sizeArtifacts` for the second pass.
export const scanArtifacts = () => call<Artifact[]>("scan_artifacts");

/// Sizes for specific artifact directories, as
/// `[path, bytes, secsSinceWrite]`.
export const sizeArtifacts = (paths: string[]) =>
  call<[string, number, number | null][]>("size_artifacts", { paths });

/// Remove artifact directories. Each is re-verified at delete time, so a
/// stale row is refused rather than acted on.
export const removeArtifacts = (paths: string[]) =>
  call<ArtifactRemoval[]>("remove_artifacts", { paths });

/// Poetry virtualenvs, classified. Discovery only: sizes and idle times
/// come from `sizeVenvs`.
export const scanVenvs = () => call<Venv[]>("scan_venvs");

/// Sizes and idle times, as `[path, bytes, idleSecs]`.
export const sizeVenvs = (paths: string[]) =>
  call<[string, number, number | null][]>("size_venvs", { paths });

/// Remove virtualenvs. Each is re-verified at delete time.
export const removeVenvs = (paths: string[]) =>
  call<VenvRemoval[]>("remove_venvs", { paths });

/// Record that a human read an assessment for this worktree.
///
/// Deliberately NOT done by `claudifyCommand`: copying a prompt is the
/// start of an assessment, and the flag this sets unlocks removing a
/// worktree past its safety gate.
export const markAssessed = (worktreePath: string) =>
  call<void>("mark_assessed", { worktreePath });

/// Forget that a worktree was assessed, restoring its Claudify action.
export const clearAssessed = (worktreePath: string) =>
  call<void>("clear_assessed", { worktreePath });

/// Run the cleanup pass now and return what it WOULD remove.
///
/// Preview only: the backend has no removal path for this, so it cannot
/// delete regardless of what it is called with.
export const previewCleanup = () => call<LedgerEntry[]>("preview_cleanup");

/// The cleanup ledger, newest first.
export const cleanupLog = () => call<LedgerEntry[]>("cleanup_log");

export const getCleanupPrefs = () => call<CleanupPrefs>("get_cleanup_prefs");
export const setCleanupPrefs = (prefs: CleanupPrefs) =>
  call<void>("set_cleanup_prefs", { prefs });

/// Which dependencies are out of date in one repository.
export const checkPackages = (repoPath: string) =>
  call<ProjectReport[]>("check_packages", { repoPath });

/// The updates as markdown, for handing to an agent.
export const packagesMarkdown = (
  repoPath: string,
  reports: ProjectReport[],
  filter: UpdateFilter,
) => call<string>("packages_markdown", { repoPath, reports, filter });

/// Create a worktree and apply updates in it. Does NOT push.
///
/// Returns where the work landed and what each update actually did.
/// Push the run's branch and open a pull request.
///


/// Reveal the diagnostic log in the file manager. Returns its path.
export const revealLog = () => call<string>("reveal_log");

/// Every CLAUDE.md in a repository, with its import tree resolved.
export const scanClaudeMd = (repoPath: string) =>
  call<ClaudeFile[]>("scan_claude_md", { repoPath });

/// The text of one file, for rendering.
export const readClaudeMd = (path: string) => call<string>("read_claude_md", { path });

/// Every branch in a repository, classified.
///
/// Slow by nature -- ~9s on a 675-branch repository, most of it the
/// patch-id comparison that finds squash merges. The caller shows a
/// loading state rather than pretending this is instant.
export const listBranches = (repoPath: string) =>
  call<Branch[]>("list_branches", { repoPath });

/// Delete local branches. Each one is re-checked at delete time.
export const deleteBranches = (repoPath: string, names: string[]) =>
  call<DeleteOutcome[]>("delete_branches", { repoPath, names });

/// Delete branches ON THE REMOTE.
///
/// Deliberately a different function from `deleteBranches`: this is a
/// push to shared state that no reflog can undo.
export const deleteRemoteBranches = (repoPath: string, names: string[]) =>
  call<DeleteOutcome[]>("delete_remote_branches", { repoPath, names });

/// Apply updates and open a pull request, in the background.
///
/// Returns as soon as the run STARTS. The outcome arrives on the
/// `update-run-done` event -- the wizard used to await the whole run
/// with its modal open, which on a large selection meant minutes of an
/// unchanging "Applying…" and an unusable app (#495).
export const applyUpdatesInBackground = (
  repoPath: string,
  requests: UpdateRequest[],
  branch?: string,
) => call<void>("apply_updates_in_background", { repoPath, requests, branch });

/// What a background update run produced.
export interface UpdateRunDone {
  repoPath: string;
  /// The pull request, when one was opened. Null means none exists —
  /// never a claim that one does.
  url: string | null;
  branch: string | null;
  applied: number;
  failed: number;
  /// Why there is no pull request, when there is none.
  error: string | null;
}

// ---------------------------------------------------------------------
// Phone pairing (mobile companion). Rust side: src-tauri/src/remote/pairing.rs
// Callers: the pairing hooks in hooks.ts, behind Settings > Phone.
// ---------------------------------------------------------------------

/// What the pairing QR code encodes. Field names are the wire format the
/// phone parses, so they stay snake_case and terse.
export interface PairingQrPayload {
  v: 1;
  name: string;
  /// Every non-loopback address of this machine, IPv4 first, overlay
  /// addresses included; the phone tries them in order.
  addrs: string[];
  port: number;
  /// `sha256:<hex>` of the desktop certificate.
  fp: string;
  /// base64url, single use, expires at `exp`.
  token: string;
  /// Unix seconds.
  exp: number;
}

/// Settings > Pair a phone. Mints a two-minute, single-use token and
/// returns what to render as a QR code. Rejects until the desktop has a
/// certificate.
export const issuePairingToken = () =>
  call<PairingQrPayload>("issue_pairing_token");

/// Payload of the `pairing-request` event: a phone has proved it holds
/// the token and is waiting on the user's decision.
export interface PairingRequest {
  request_id: number;
  device_name: string;
  /// Lowercase hex, no prefix. Show it in blocks of four; the phone shows
  /// the same string so the two can be compared.
  fingerprint: string;
  /// Whether the phone offered a post-quantum step-up key.
  has_mldsa: boolean;
}

/// Answer a `pairing-request`.
///
/// `replaceExisting` matters only when a device with the same name is
/// already paired (check `listPairedDevices` when the event arrives):
/// `true` replaces it, `false` keeps both, and leaving it out rejects
/// with a message naming the device while the request stays pending --
/// so the modal can ask "replace or keep both?" and answer again. The
/// desktop never picks either on its own.
export const respondToPairing = (
  requestId: number,
  approve: boolean,
  replaceExisting?: boolean,
) =>
  call<void>("respond_to_pairing", {
    requestId,
    approve,
    replaceExisting: replaceExisting ?? null,
  });

/// A paired phone as Settings lists it. No key material.
export interface PairedDevice {
  id: number;
  name: string;
  /// Lowercase hex, no prefix.
  cert_fp: string;
  has_mldsa: boolean;
  /// RFC 3339.
  paired_at: string;
  /// RFC 3339, or null until the device's first connection after pairing.
  last_seen: string | null;
}

export const listPairedDevices = () =>
  call<PairedDevice[]>("list_paired_devices");

/// Delete the row and close that phone's open connections. A second
/// click on an already-revoked device resolves rather than rejects.
export const revokePairedDevice = (id: number) =>
  call<void>("revoke_paired_device", { id });
