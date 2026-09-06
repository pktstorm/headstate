//! The remote allowlist, as the phone knows it.
//!
//! A copy of [`SURFACE`] from the desktop's `src-tauri/src/remote/surface.rs`,
//! so that `remote_call` refuses a command the desktop would refuse
//! BEFORE putting it on the wire, and so the phone knows which commands
//! need the step-up signature. The spec asks for this client-side check
//! so a mistake in the frontend fails locally with a clear message rather
//! than with a 404 from the desktop.
//!
//! Two copies of one table is a drift risk. It is held together by
//! [`tests::table_is_identical_to_the_desktop_table`], which reads the
//! desktop's source file at test time and compares row by row, in
//! order: a class change or an added command on either side fails the
//! mobile tests until the other side is updated.

/// What a command does. Same four classes as the desktop, same meaning.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Class {
    /// No side effects on GitHub or disk.
    Read,
    /// Changes GitHub state or a desktop setting.
    Write,
    /// Deletes files, branches, images, or volumes. Carries the step-up
    /// signature (`stepup.rs`).
    Destructive,
    /// Desktop-only; refused here and there.
    Local,
}

/// Command name to class, in the desktop's order.
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
    ("issue_pairing_token", Class::Local),
    ("respond_to_pairing", Class::Local),
    ("list_paired_devices", Class::Local),
    ("revoke_paired_device", Class::Local),
    ("get_remote_enabled", Class::Local),
    ("set_remote_enabled", Class::Local),
];

/// The class of a command, or `None` when the desktop has no such
/// command.
pub fn class_of(command: &str) -> Option<Class> {
    SURFACE
        .iter()
        .find(|(name, _)| *name == command)
        .map(|(_, class)| *class)
}

/// Why `remote_call` did not put a command on the wire. The messages are
/// what the frontend sees as the rejection reason.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum Refusal {
    #[error("`{0}` is not a Headstate command")]
    Unknown(String),
    #[error("`{0}` is only available on the desktop")]
    Local(String),
}

/// Known and not local, or the refusal naming why.
pub fn admit(command: &str) -> Result<Class, Refusal> {
    match class_of(command) {
        None => Err(Refusal::Unknown(command.to_string())),
        Some(Class::Local) => Err(Refusal::Local(command.to_string())),
        Some(class) => Ok(class),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The desktop's table, parsed from its source. `include_str!` ties
    /// this test to the desktop file at compile time, so the two are
    /// compared as they are checked in, not as someone remembers them.
    fn desktop_table() -> Vec<(String, String)> {
        let src = include_str!("../../src-tauri/src/remote/surface.rs");
        let start = src
            .find("pub const SURFACE")
            .expect("desktop surface.rs must define SURFACE");
        let body = &src[start..];
        let end = body.find("];").expect("SURFACE must close");
        body[..end]
            .lines()
            .filter_map(|line| {
                let line = line.trim();
                let rest = line.strip_prefix("(\"")?;
                let (name, rest) = rest.split_once("\", Class::")?;
                let class = rest.trim_end_matches("),");
                Some((name.to_string(), class.to_string()))
            })
            .collect()
    }

    fn class_name(class: Class) -> &'static str {
        match class {
            Class::Read => "Read",
            Class::Write => "Write",
            Class::Destructive => "Destructive",
            Class::Local => "Local",
        }
    }

    #[test]
    fn table_is_identical_to_the_desktop_table() {
        let desktop = desktop_table();
        assert!(
            desktop.len() > 50,
            "parsed only {} rows from the desktop's surface.rs; the parser is broken",
            desktop.len()
        );
        let mobile: Vec<(String, String)> = SURFACE
            .iter()
            .map(|(name, class)| (name.to_string(), class_name(*class).to_string()))
            .collect();
        assert_eq!(
            mobile, desktop,
            "src-mobile/src/surface.rs SURFACE differs from src-tauri/src/remote/surface.rs; \
             copy the desktop's table verbatim"
        );
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
    fn local_and_unknown_commands_are_refused_with_the_desktop_wording() {
        assert_eq!(
            admit("reveal_log"),
            Err(Refusal::Local("reveal_log".into()))
        );
        assert_eq!(
            admit("reveal_log").unwrap_err().to_string(),
            "`reveal_log` is only available on the desktop"
        );
        assert_eq!(
            admit("getCached"),
            Err(Refusal::Unknown("getCached".into()))
        );
        assert_eq!(admit("get_cached"), Ok(Class::Read));
        assert_eq!(admit("remove_worktree"), Ok(Class::Destructive));
    }
}
