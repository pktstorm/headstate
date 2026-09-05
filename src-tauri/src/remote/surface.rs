//! The remote allowlist: the single place that decides what a paired
//! phone can do.
//!
//! Every Tauri command registered in `lib.rs`'s `generate_handler!` has
//! exactly one row in [`SURFACE`], and the row's [`Class`] is the whole
//! policy. A command added to `generate_handler!` without a row here
//! fails [`tests::every_registered_command_has_exactly_one_class`], so a
//! new command can be neither exposed nor omitted silently.
//!
//! # Contract for the `POST /v1/call/{command}` handler
//!
//! The HTTP route lives in `remote/listener.rs`, not here. The handler
//! that mounts it must:
//!
//! 1. Read `{command}` from the path and the JSON body as
//!    `serde_json::Value` (an object of camelCase keys, exactly what the
//!    webview passes to `invoke`; an empty or absent body is `{}`).
//! 2. Look the command up with [`class_of`]. When it is
//!    [`Class::Destructive`], verify the `X-Headstate-Signature` step-up
//!    header (nonce, timestamp, and every signature the pairing record
//!    expects) BEFORE calling [`dispatch`]. `dispatch` does not check
//!    signatures; it trusts that the caller already refused a destructive
//!    request without a valid one.
//! 3. Call [`dispatch`] with the paired device's name and map the result:
//!    `Ok(value)` is the response body, and each [`RemoteError`] variant
//!    documents the status it should become.
//!
//! `dispatch` refuses [`Class::Local`] and unknown commands itself, so a
//! handler that skips step 2 for a non-destructive command is still safe;
//! it is the signature check that only the handler can do.

use serde::de::DeserializeOwned;
use serde::Serialize;
use serde_json::Value;
use tauri::{AppHandle, Manager};

use crate::commands;

/// What a command does, which decides what a phone must present to run it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Class {
    /// No side effects on GitHub or disk.
    Read,
    /// Changes GitHub state through the existing write module, or a
    /// desktop setting.
    Write,
    /// Deletes files, branches, images, or volumes. Requires the step-up
    /// signature (see the module docs) before dispatch.
    Destructive,
    /// Not exposed remotely: opens a window, reveals a file, changes
    /// autostart, restarts a daemon, runs an agent, or steers the
    /// desktop's own view.
    Local,
}

/// Command name to class. The order is the spec's; keep it that way so a
/// diff against the design document is a line-by-line comparison.
pub const SURFACE: &[(&str, Class)] = &[
    // read: no side effects on GitHub or disk.
    ("get_auth_state", Class::Read),
    ("get_cached", Class::Read),
    ("get_cached_reviewing", Class::Read),
    ("refresh_now", Class::Read),
    ("get_stats", Class::Read),
    ("get_history", Class::Read),
    ("get_periods", Class::Read),
    ("get_cycle_trend", Class::Read),
    ("get_merged_detail", Class::Read),
    ("get_reviewing", Class::Read),
    ("count_reviewing", Class::Read),
    ("get_pr_detail", Class::Read),
    ("get_viewer", Class::Read),
    ("build_target", Class::Read),
    ("latest_release", Class::Read),
    ("list_worktrees", Class::Read),
    ("classify_worktrees", Class::Read),
    ("size_worktrees", Class::Read),
    ("list_branches", Class::Read),
    ("scan_artifacts", Class::Read),
    ("size_artifacts", Class::Read),
    ("scan_venvs", Class::Read),
    ("size_venvs", Class::Read),
    ("docker_state", Class::Read),
    ("docker_builds", Class::Read),
    ("docker_images", Class::Read),
    ("docker_disk_usage", Class::Read),
    ("docker_dangling_volumes", Class::Read),
    ("docker_running_containers", Class::Read),
    ("preview_cleanup", Class::Read),
    ("cleanup_log", Class::Read),
    ("get_cleanup_prefs", Class::Read),
    ("assessed_worktrees", Class::Read),
    ("check_packages", Class::Read),
    ("packages_markdown", Class::Read),
    ("scan_claude_md", Class::Read),
    ("read_claude_md", Class::Read),
    ("get_poll_interval", Class::Read),
    ("get_worktree_dirs", Class::Read),
    // write: changes GitHub state through the existing write module, or
    // a desktop setting.
    ("act_on_pr", Class::Write),
    ("act_on_prs", Class::Write),
    ("review_pr", Class::Write),
    ("comment_on_pr", Class::Write),
    ("resolve_thread", Class::Write),
    ("unresolve_thread", Class::Write),
    ("reply_to_thread", Class::Write),
    ("rerun_checks", Class::Write),
    ("update_pr_branch", Class::Write),
    ("set_auto_merge", Class::Write),
    ("mark_assessed", Class::Write),
    ("clear_assessed", Class::Write),
    ("set_cleanup_prefs", Class::Write),
    ("set_poll_interval", Class::Write),
    ("open_update_pr", Class::Write),
    // destructive: deletes files, branches, images, or volumes.
    ("delete_head_branch", Class::Destructive),
    ("delete_branches", Class::Destructive),
    ("delete_remote_branches", Class::Destructive),
    ("remove_worktree", Class::Destructive),
    ("remove_worktrees", Class::Destructive),
    ("remove_worktree_forced", Class::Destructive),
    ("remove_artifacts", Class::Destructive),
    ("remove_venvs", Class::Destructive),
    ("remove_orphan", Class::Destructive),
    ("docker_remove_images", Class::Destructive),
    ("docker_remove_volume", Class::Destructive),
    ("docker_prune_cache", Class::Destructive),
    ("apply_package_updates", Class::Destructive),
    // local: not exposed remotely.
    ("diag_log", Class::Local),
    ("reveal_log", Class::Local),
    ("pull_checkout", Class::Local),
    ("get_ui_prefs", Class::Local),
    ("set_ui_prefs", Class::Local),
    ("get_autostart", Class::Local),
    ("set_autostart", Class::Local),
    ("get_notify_prefs", Class::Local),
    ("set_notify_prefs", Class::Local),
    ("set_worktree_dirs", Class::Local),
    ("assess_worktree", Class::Local),
    ("claudify_command", Class::Local),
    ("apply_updates_in_background", Class::Local),
    ("docker_restart", Class::Local),
    ("docker_start", Class::Local),
    ("set_view_needs_github", Class::Local),
    // The remote feature's own commands. Pairing and the on/off switch
    // are decisions the desktop's user makes at the desktop: a phone
    // that could approve its own pairing request, revoke a rival, or
    // turn the listener off would defeat the point of each.
    ("issue_pairing_token", Class::Local),
    ("respond_to_pairing", Class::Local),
    ("list_paired_devices", Class::Local),
    ("revoke_paired_device", Class::Local),
    ("get_remote_enabled", Class::Local),
    ("set_remote_enabled", Class::Local),
];

/// The class of a registered command, or `None` when no such command
/// exists.
pub fn class_of(command: &str) -> Option<Class> {
    SURFACE
        .iter()
        .find(|(name, _)| *name == command)
        .map(|(_, class)| *class)
}

/// Why a remote call was not carried out, or did not succeed.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum RemoteError {
    /// No registered command has this name. Suggested status: 404.
    #[error("`{0}` is not a Headstate command")]
    Unknown(String),
    /// The command exists but is desktop-only. Suggested status: 403.
    #[error("`{0}` is only available on the desktop")]
    Local(String),
    /// The body did not decode into the command's arguments. Suggested
    /// status: 400.
    #[error("bad arguments for `{command}`: {message}")]
    BadArgs { command: String, message: String },
    /// The command ran and returned its own `Err(String)`; the message is
    /// verbatim what the webview would have seen as the rejection reason.
    /// Suggested status: 500.
    #[error("{0}")]
    Command(String),
}

impl RemoteError {
    /// The statuses suggested on each variant, for the `/v1/call`
    /// handler in `remote/listener.rs`.
    pub fn http_status(&self) -> u16 {
        match self {
            RemoteError::Unknown(_) => 404,
            RemoteError::Local(_) => 403,
            RemoteError::BadArgs { .. } => 400,
            RemoteError::Command(_) => 500,
        }
    }
}

/// The gate `dispatch` applies before touching any argument: known and
/// not local, or a refusal naming why. Separate from `dispatch` so the
/// refusals are testable without an `AppHandle`.
fn admit(command: &str) -> Result<Class, RemoteError> {
    match class_of(command) {
        None => Err(RemoteError::Unknown(command.to_string())),
        Some(Class::Local) => Err(RemoteError::Local(command.to_string())),
        Some(class) => Ok(class),
    }
}

/// The JSON body of a call, read the way Tauri's IPC reads it: one key per
/// argument, camelCase, with a missing key acceptable only for an
/// `Option` argument.
struct Args<'a> {
    command: &'a str,
    body: Value,
}

impl<'a> Args<'a> {
    fn new(command: &'a str, body: Value) -> Result<Self, RemoteError> {
        match body {
            Value::Object(_) => Ok(Self { command, body }),
            Value::Null => Ok(Self {
                command,
                body: Value::Object(Default::default()),
            }),
            other => Err(RemoteError::BadArgs {
                command: command.to_string(),
                message: format!("expected a JSON object of arguments, got {other}"),
            }),
        }
    }

    /// One argument by its camelCase key. A missing key decodes as JSON
    /// `null`, which is `None` for an `Option<T>` and an error naming the
    /// key for anything else, matching what Tauri reports.
    fn get<T: DeserializeOwned>(&self, key: &str) -> Result<T, RemoteError> {
        let value = self.body.get(key).cloned().unwrap_or(Value::Null);
        serde_json::from_value(value).map_err(|e| RemoteError::BadArgs {
            command: self.command.to_string(),
            message: if self.body.get(key).is_none() {
                format!("missing required argument `{key}`")
            } else {
                format!("argument `{key}`: {e}")
            },
        })
    }
}

/// A command's plain return value, as the webview would receive it.
fn ok<T: Serialize>(value: T) -> Result<Value, RemoteError> {
    serde_json::to_value(value).map_err(|e| RemoteError::Command(e.to_string()))
}

/// A command's `Result<T, String>`, as the webview would receive it.
fn res<T: Serialize>(result: Result<T, String>) -> Result<Value, RemoteError> {
    result.map_err(RemoteError::Command).and_then(ok)
}

/// Run one allowlisted command on behalf of a paired device.
///
/// Refuses [`Class::Local`] and unknown commands, decodes `args` into the
/// same argument shapes the webview sends, and calls the same
/// `commands::*` function the webview would have called, with the same
/// managed state. Each command keeps its own logging; this adds one line
/// per call naming the device, so a log reads "phone asked, desktop did".
///
/// Does NOT verify the step-up signature for destructive commands. The
/// HTTP handler must do that first; see the module docs.
pub async fn dispatch(
    app: &AppHandle,
    command: &str,
    args: Value,
    device_name: &str,
) -> Result<Value, RemoteError> {
    let class = admit(command)?;
    log::info!("remote: {device_name} called {command} ({class:?})");
    let result = call(app, command, Args::new(command, args)?).await;
    if let Err(e) = &result {
        log::warn!("remote: {command} for {device_name} failed: {e}");
    }
    result
}

/// The match over the allowlist. Every non-local row of [`SURFACE`] has an
/// arm here, which [`tests::every_remote_command_has_a_dispatch_arm`]
/// enforces on the source.
///
/// Managed state is reached through `app.state()`, which yields the same
/// `State<'_, T>` Tauri injects into the command; `AppHandle` arguments
/// get a clone of `app`. Argument keys are the camelCase names the
/// webview sends, so `repo_path` on the Rust side is `"repoPath"` here.
async fn call(app: &AppHandle, command: &str, a: Args<'_>) -> Result<Value, RemoteError> {
    match command {
        // ---- read -------------------------------------------------------
        "get_auth_state" => ok(commands::get_auth_state(app.state())),
        "get_cached" => res(commands::get_cached(app.clone())),
        "get_cached_reviewing" => res(commands::get_cached_reviewing(app.clone())),
        "refresh_now" => res(commands::refresh_now(app.state()).await),
        "get_stats" => res(commands::get_stats(app.state()).await),
        "get_history" => res(commands::get_history(app.state(), a.get("days")?).await),
        "get_periods" => res(commands::get_periods(app.state()).await),
        "get_cycle_trend" => res(commands::get_cycle_trend(app.state()).await),
        "get_merged_detail" => res(commands::get_merged_detail(app.state()).await),
        "get_reviewing" => res(commands::get_reviewing(app.clone(), app.state()).await),
        "count_reviewing" => res(commands::count_reviewing(app.state()).await),
        "get_pr_detail" => {
            res(commands::get_pr_detail(app.state(), a.get("repo")?, a.get("number")?).await)
        }
        "get_viewer" => res(commands::get_viewer(app.state()).await),
        "build_target" => ok(commands::build_target()),
        "latest_release" => ok(commands::latest_release(app.clone()).await),
        "list_worktrees" => res(commands::list_worktrees(app.clone()).await),
        "classify_worktrees" => res(commands::classify_worktrees(a.get("repoPath")?).await),
        "size_worktrees" => res(commands::size_worktrees(a.get("repoPath")?).await),
        "list_branches" => res(commands::list_branches(a.get("repoPath")?).await),
        "scan_artifacts" => res(commands::scan_artifacts(app.clone()).await),
        "size_artifacts" => res(commands::size_artifacts(a.get("paths")?).await),
        "scan_venvs" => res(commands::scan_venvs(app.clone()).await),
        "size_venvs" => res(commands::size_venvs(a.get("paths")?).await),
        "docker_state" => ok(commands::docker_state().await),
        "docker_builds" => res(commands::docker_builds().await),
        "docker_images" => res(commands::docker_images(app.clone()).await),
        "docker_disk_usage" => res(commands::docker_disk_usage().await),
        // The sync Docker commands shell out. Inline they would stall the
        // listener's worker for every other request (#496 was this bug
        // in the webview); the blocking pool is where they belong.
        "docker_dangling_volumes" => res(blocking(commands::docker_dangling_volumes).await?),
        "docker_running_containers" => res(blocking(commands::docker_running_containers).await?),
        "preview_cleanup" => res(commands::preview_cleanup(app.clone()).await),
        "cleanup_log" => res(commands::cleanup_log(app.clone())),
        "get_cleanup_prefs" => ok(commands::get_cleanup_prefs(app.clone())),
        "assessed_worktrees" => ok(commands::assessed_worktrees(app.clone())),
        "check_packages" => res(commands::check_packages(a.get("repoPath")?).await),
        "packages_markdown" => ok(commands::packages_markdown(
            a.get("repoPath")?,
            a.get("reports")?,
            a.get("filter")?,
        )),
        "scan_claude_md" => res(commands::scan_claude_md(a.get("repoPath")?).await),
        "read_claude_md" => res(commands::read_claude_md(a.get("path")?)),
        "get_poll_interval" => ok(commands::get_poll_interval(app.state())),
        "get_worktree_dirs" => ok(commands::get_worktree_dirs(app.clone())),

        // ---- write ------------------------------------------------------
        "act_on_pr" => res(commands::act_on_pr(
            app.state(),
            app.state(),
            a.get("id")?,
            a.get("repo")?,
            a.get("number")?,
            a.get("action")?,
        )
        .await),
        "act_on_prs" => {
            res(
                commands::act_on_prs(app.state(), app.state(), a.get("prs")?, a.get("action")?)
                    .await,
            )
        }
        "review_pr" => res(commands::review_pr(
            app.state(),
            app.state(),
            a.get("id")?,
            a.get("repo")?,
            a.get("number")?,
            a.get("verdict")?,
            a.get("body")?,
        )
        .await),
        "comment_on_pr" => res(commands::comment_on_pr(
            app.state(),
            a.get("id")?,
            a.get("repo")?,
            a.get("number")?,
            a.get("body")?,
        )
        .await),
        "resolve_thread" => res(commands::resolve_thread(
            app.state(),
            a.get("threadId")?,
            a.get("repo")?,
            a.get("number")?,
        )
        .await),
        "unresolve_thread" => res(commands::unresolve_thread(
            app.state(),
            a.get("threadId")?,
            a.get("repo")?,
            a.get("number")?,
        )
        .await),
        "reply_to_thread" => res(commands::reply_to_thread(
            app.state(),
            a.get("threadId")?,
            a.get("repo")?,
            a.get("number")?,
            a.get("body")?,
        )
        .await),
        "rerun_checks" => res(commands::rerun_checks(
            app.state(),
            app.state(),
            a.get("repo")?,
            a.get("number")?,
            a.get("runId")?,
        )
        .await),
        "update_pr_branch" => res(commands::update_pr_branch(
            app.state(),
            app.state(),
            a.get("id")?,
            a.get("repo")?,
            a.get("number")?,
            a.get("expectedHead")?,
        )
        .await),
        "set_auto_merge" => res(commands::set_auto_merge(
            app.state(),
            app.state(),
            a.get("id")?,
            a.get("repo")?,
            a.get("number")?,
            a.get("expectedHead")?,
            a.get("enable")?,
        )
        .await),
        "mark_assessed" => res(commands::mark_assessed(app.clone(), a.get("worktreePath")?)),
        "clear_assessed" => res(commands::clear_assessed(
            app.clone(),
            a.get("worktreePath")?,
        )),
        "set_cleanup_prefs" => res(commands::set_cleanup_prefs(app.clone(), a.get("prefs")?)),
        "set_poll_interval" => ok(commands::set_poll_interval(
            app.clone(),
            a.get("secs")?,
            app.state(),
            app.state(),
        )),
        "open_update_pr" => {
            res(commands::open_update_pr(app.state(), a.get("repoPath")?, a.get("report")?).await)
        }

        // ---- destructive (signature already verified by the handler) ----
        "delete_head_branch" => res(commands::delete_head_branch(
            app.state(),
            app.state(),
            a.get("refId")?,
            a.get("repo")?,
            a.get("number")?,
            a.get("branch")?,
            a.get("merged")?,
        )
        .await),
        "delete_branches" => {
            res(commands::delete_branches(a.get("repoPath")?, a.get("names")?).await)
        }
        "delete_remote_branches" => {
            res(commands::delete_remote_branches(a.get("repoPath")?, a.get("names")?).await)
        }
        "remove_worktree" => {
            res(commands::remove_worktree(a.get("repoPath")?, a.get("worktreePath")?).await)
        }
        "remove_worktrees" => res(commands::remove_worktrees(
            app.clone(),
            a.get("repoPath")?,
            a.get("worktreePaths")?,
        )
        .await),
        "remove_worktree_forced" => res(commands::remove_worktree_forced(
            app.clone(),
            a.get("repoPath")?,
            a.get("worktreePath")?,
        )
        .await),
        "remove_artifacts" => res(commands::remove_artifacts(app.clone(), a.get("paths")?).await),
        "remove_venvs" => res(commands::remove_venvs(app.clone(), a.get("paths")?).await),
        "remove_orphan" => res(commands::remove_orphan(a.get("path")?).await),
        "docker_remove_images" => {
            let ids: Vec<String> = a.get("ids")?;
            ok(blocking(move || commands::docker_remove_images(ids)).await?)
        }
        "docker_remove_volume" => {
            let name: String = a.get("name")?;
            res(blocking(move || commands::docker_remove_volume(name)).await?)
        }
        "docker_prune_cache" => {
            let until: Option<String> = a.get("until")?;
            res(blocking(move || commands::docker_prune_cache(until)).await?)
        }
        "apply_package_updates" => {
            res(commands::apply_package_updates(a.get("repoPath")?, a.get("requests")?).await)
        }

        // A classified, non-local command with no arm is a wiring bug
        // that the source test catches; at runtime it must still refuse
        // rather than pretend.
        _ => Err(RemoteError::Unknown(command.to_string())),
    }
}

/// Run a synchronous, shelling-out command on the blocking pool.
async fn blocking<T, F>(f: F) -> Result<T, RemoteError>
where
    T: Send + 'static,
    F: FnOnce() -> T + Send + 'static,
{
    tauri::async_runtime::spawn_blocking(f)
        .await
        .map_err(|e| RemoteError::Command(format!("command task failed: {e}")))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    /// The `path::name` entries inside `generate_handler![...]` in
    /// `lib.rs`, read from the source at test time so the desktop's real
    /// registration is what is compared, not a copy of it. Any module
    /// path counts -- `commands::x` and `remote::pairing::x` alike --
    /// because a command registered under a path the parser ignored
    /// would be neither classified nor caught.
    fn registered_commands() -> Vec<String> {
        let src = include_str!("../lib.rs");
        let open = "generate_handler![";
        let start = src
            .find(open)
            .expect("lib.rs must register commands with generate_handler!");
        let block = &src[start + open.len()..];
        let end = block.find(']').expect("generate_handler! block must close");
        block[..end]
            .split_whitespace()
            .map(|tok| tok.trim_end_matches(','))
            .filter(|tok| tok.contains("::"))
            .filter_map(|tok| tok.rsplit("::").next())
            .map(str::to_string)
            .collect()
    }

    #[test]
    fn the_parser_sees_commands_under_every_module_path() {
        let registered = registered_commands();
        for name in ["get_cached", "issue_pairing_token", "set_remote_enabled"] {
            assert!(registered.iter().any(|r| r == name), "{name} not parsed");
        }
    }

    #[test]
    fn every_registered_command_has_exactly_one_class() {
        let registered = registered_commands();
        assert!(
            registered.len() > 50,
            "parsed only {} commands from lib.rs; the parser is broken",
            registered.len()
        );
        let mut problems = Vec::new();
        for name in &registered {
            let rows = SURFACE.iter().filter(|(n, _)| n == name).count();
            if rows != 1 {
                problems.push(format!(
                    "{name}: registered in lib.rs but has {rows} rows in SURFACE (need exactly 1)"
                ));
            }
        }
        for (name, _) in SURFACE {
            if !registered.iter().any(|r| r == name) {
                problems.push(format!(
                    "{name}: classified in SURFACE but not registered in lib.rs"
                ));
            }
        }
        assert!(problems.is_empty(), "{}", problems.join("\n"));
    }

    #[test]
    fn class_of_reports_each_class_and_nothing_for_unknown_names() {
        assert_eq!(class_of("get_cached"), Some(Class::Read));
        assert_eq!(class_of("act_on_pr"), Some(Class::Write));
        assert_eq!(class_of("remove_worktree"), Some(Class::Destructive));
        assert_eq!(class_of("reveal_log"), Some(Class::Local));
        assert_eq!(class_of("drop_database"), None);
    }

    #[test]
    fn local_commands_are_refused() {
        for name in [
            "reveal_log",
            "pull_checkout",
            "set_autostart",
            "assess_worktree",
            "docker_restart",
            "issue_pairing_token",
            "respond_to_pairing",
            "list_paired_devices",
            "revoke_paired_device",
            "get_remote_enabled",
            "set_remote_enabled",
        ] {
            assert_eq!(
                admit(name),
                Err(RemoteError::Local(name.to_string())),
                "{name} must be refused as local"
            );
        }
    }

    #[test]
    fn each_error_maps_to_the_status_its_docs_suggest() {
        assert_eq!(RemoteError::Unknown("x".into()).http_status(), 404);
        assert_eq!(RemoteError::Local("x".into()).http_status(), 403);
        let bad = RemoteError::BadArgs {
            command: "x".into(),
            message: "m".into(),
        };
        assert_eq!(bad.http_status(), 400);
        assert_eq!(RemoteError::Command("m".into()).http_status(), 500);
    }

    #[test]
    fn unknown_commands_are_refused() {
        for name in ["", "drop_database", "getCached", "commands::get_cached"] {
            assert_eq!(admit(name), Err(RemoteError::Unknown(name.to_string())));
        }
    }

    #[test]
    fn read_write_and_destructive_commands_are_admitted() {
        assert_eq!(admit("get_cached"), Ok(Class::Read));
        assert_eq!(admit("act_on_pr"), Ok(Class::Write));
        assert_eq!(admit("remove_worktree"), Ok(Class::Destructive));
    }

    /// Every non-local row must have a `"name" =>` arm in `call`. Asserted
    /// on the source because `call` needs a live `AppHandle`, which a unit
    /// test cannot construct for the Wry runtime the commands are typed
    /// against.
    #[test]
    fn every_remote_command_has_a_dispatch_arm() {
        let src = include_str!("surface.rs");
        let start = src.find("async fn call(").expect("call must exist");
        let end = src[start..]
            .find("#[cfg(test)]")
            .map(|i| start + i)
            .unwrap_or(src.len());
        let body = &src[start..end];
        let missing: Vec<&str> = SURFACE
            .iter()
            .filter(|(_, class)| *class != Class::Local)
            .map(|(name, _)| *name)
            .filter(|name| !body.contains(&format!("\"{name}\" =>")))
            .collect();
        assert!(
            missing.is_empty(),
            "classified as remote but not dispatched: {}",
            missing.join(", ")
        );
        let local_with_arm: Vec<&str> = SURFACE
            .iter()
            .filter(|(_, class)| *class == Class::Local)
            .map(|(name, _)| *name)
            .filter(|name| body.contains(&format!("\"{name}\" =>")))
            .collect();
        assert!(
            local_with_arm.is_empty(),
            "local commands must not have an arm: {}",
            local_with_arm.join(", ")
        );
    }

    /// The webview sends camelCase keys (`repoPath`), and Tauri's default
    /// argument renaming expects them. A snake_case key in an arm would
    /// compile and then fail every call with "missing required argument".
    #[test]
    fn dispatch_arms_read_camel_case_keys_only() {
        let src = include_str!("surface.rs");
        let start = src.find("async fn call(").expect("call must exist");
        let end = src[start..].find("#[cfg(test)]").unwrap() + start;
        let snake: Vec<&str> = src[start..end]
            .split(".get(\"")
            .skip(1)
            .filter_map(|rest| rest.split('"').next())
            .filter(|key| key.contains('_'))
            .collect();
        assert!(snake.is_empty(), "snake_case argument keys: {snake:?}");
    }

    // One command per class, decoded from exactly the body its `tauri.ts`
    // wrapper sends.

    #[test]
    fn read_args_decode_as_the_webview_sends_them() {
        let a = Args::new("get_history", json!({ "days": 14 })).unwrap();
        assert_eq!(a.get::<i64>("days"), Ok(14));
    }

    #[test]
    fn write_args_decode_as_the_webview_sends_them() {
        let a = Args::new(
            "act_on_pr",
            json!({ "id": "PR_1", "repo": "octocat/hello-world", "number": 7, "action": "merge" }),
        )
        .unwrap();
        assert_eq!(a.get::<String>("id"), Ok("PR_1".into()));
        assert_eq!(a.get::<String>("repo"), Ok("octocat/hello-world".into()));
        assert_eq!(a.get::<u64>("number"), Ok(7));
        assert_eq!(a.get::<String>("action"), Ok("merge".into()));
    }

    #[test]
    fn destructive_args_decode_as_the_webview_sends_them() {
        let a = Args::new(
            "remove_worktrees",
            json!({ "repoPath": "/srv/hello-world", "worktreePaths": ["/srv/hello-world-wt"] }),
        )
        .unwrap();
        assert_eq!(a.get::<String>("repoPath"), Ok("/srv/hello-world".into()));
        assert_eq!(
            a.get::<Vec<String>>("worktreePaths"),
            Ok(vec!["/srv/hello-world-wt".to_string()])
        );
    }

    #[test]
    fn a_missing_required_argument_names_the_key() {
        let a = Args::new("remove_worktree", json!({ "repoPath": "/srv/x" })).unwrap();
        assert_eq!(
            a.get::<String>("worktreePath"),
            Err(RemoteError::BadArgs {
                command: "remove_worktree".into(),
                message: "missing required argument `worktreePath`".into(),
            })
        );
    }

    #[test]
    fn a_missing_optional_argument_is_none() {
        let a = Args::new("docker_prune_cache", json!({})).unwrap();
        assert_eq!(a.get::<Option<String>>("until"), Ok(None));
        let a = Args::new("docker_prune_cache", json!({ "until": "24h" })).unwrap();
        assert_eq!(a.get::<Option<String>>("until"), Ok(Some("24h".into())));
    }

    #[test]
    fn a_wrongly_typed_argument_is_refused_with_the_key() {
        let a = Args::new("get_history", json!({ "days": "fourteen" })).unwrap();
        let err = a.get::<i64>("days").unwrap_err();
        match err {
            RemoteError::BadArgs { command, message } => {
                assert_eq!(command, "get_history");
                assert!(message.starts_with("argument `days`:"), "{message}");
            }
            other => panic!("expected BadArgs, got {other:?}"),
        }
    }

    #[test]
    fn a_null_body_is_an_empty_argument_set_and_a_non_object_is_refused() {
        assert!(Args::new("get_cached", Value::Null).is_ok());
        assert!(Args::new("get_cached", json!([1, 2])).is_err());
        assert!(Args::new("get_cached", json!("x")).is_err());
    }

    #[test]
    fn a_command_error_is_passed_through_verbatim() {
        let r: Result<(), String> = Err(commands::AUTH_ERR.to_string());
        assert_eq!(res(r), Err(RemoteError::Command(commands::AUTH_ERR.into())));
        assert_eq!(res(Ok(("a".to_string(), 1u64))), Ok(json!(["a", 1])));
    }
}
