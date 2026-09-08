//! What background update runs are in flight, and how they end.
//!
//! `apply_updates_in_background` returns immediately by design (#495):
//! a run is one package-manager invocation per package, and a selection
//! of 122 takes minutes. That leaves three things nobody was tracking.
//!
//! **Nothing could stop a run.** The spawned task's `JoinHandle` was
//! dropped, so a run that was started by mistake -- wrong branch, wrong
//! selection -- ran to completion regardless.
//!
//! **Nothing stopped a second run** starting on a repository already
//! updating, which would put two package managers in the same worktree.
//!
//! **A phone could not learn how a run ended.** Progress and completion
//! are events, and a suspended app holds no event stream at all
//! (`src-mobile/src/background.rs` is explicit about this). So a phone
//! that started a run and went to sleep missed every frame, including
//! the terminal one, and had no way to ask. That is the difference
//! between a feature a phone can use and one it can only start.
//!
//! This registry answers all three, keyed by repository path because
//! that is what a run acts on and what a returning client knows.

use crate::commands::UpdateRunDone;
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

/// A run in flight, or the outcome of the last one to finish.
///
/// Both live in the same map: a client asking "what happened to my run"
/// gets one answer whether the run is still going or ended while it was
/// away, and does not have to ask twice.
#[derive(Debug, Clone, serde::Serialize)]
#[serde(tag = "state", rename_all = "lowercase")]
pub enum RunState {
    /// Still going. `done` counts finished packages.
    Running { done: usize, total: usize },
    /// Ended, with the same payload `update-run-done` carried. A client
    /// that missed the event reads it here instead.
    Done { outcome: Box<UpdateRunDone> },
}

/// One repository's run.
struct Entry {
    /// Set by `cancel`, read between packages by the run itself.
    stop: Arc<AtomicBool>,
    state: RunState,
}

/// Every run this process has started, by repository path.
///
/// Bounded by the number of repositories someone has updated this
/// session, and each entry is a handful of integers plus one outcome --
/// so it is not worth evicting, and evicting would defeat the resume
/// read it exists for.
#[derive(Default)]
pub struct UpdateRuns(Mutex<HashMap<String, Entry>>);

impl UpdateRuns {
    /// Claim a repository for a new run.
    ///
    /// `Err` when one is already going: two package managers in one
    /// worktree is not something to discover afterwards. The message is
    /// user-facing, since this is what the command returns.
    pub fn start(&self, repo: &str, total: usize) -> Result<Arc<AtomicBool>, String> {
        let mut runs = self.0.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(e) = runs.get(repo) {
            if matches!(e.state, RunState::Running { .. }) {
                return Err("an update run is already going in this repository".into());
            }
        }
        let stop = Arc::new(AtomicBool::new(false));
        runs.insert(
            repo.to_string(),
            Entry {
                stop: stop.clone(),
                state: RunState::Running { done: 0, total },
            },
        );
        Ok(stop)
    }

    /// Record progress. Ignored once the run has ended, so a late frame
    /// cannot resurrect a finished run as `Running`.
    pub fn progress(&self, repo: &str, done: usize, total: usize) {
        let mut runs = self.0.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(e) = runs.get_mut(repo) {
            if matches!(e.state, RunState::Running { .. }) {
                e.state = RunState::Running { done, total };
            }
        }
    }

    /// Record how a run ended. The entry stays, so a client that was
    /// away can still read it.
    pub fn finished(&self, repo: &str, outcome: UpdateRunDone) {
        let mut runs = self.0.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(e) = runs.get_mut(repo) {
            e.state = RunState::Done {
                outcome: Box::new(outcome),
            };
        }
    }

    /// Ask a run to stop after its current package.
    ///
    /// `Err` when there is nothing running, rather than succeeding
    /// silently: a Cancel that appears to work on a run that already
    /// finished would be its own small lie.
    pub fn cancel(&self, repo: &str) -> Result<(), String> {
        let runs = self.0.lock().unwrap_or_else(|e| e.into_inner());
        match runs.get(repo) {
            Some(e) if matches!(e.state, RunState::Running { .. }) => {
                e.stop.store(true, Ordering::SeqCst);
                Ok(())
            }
            _ => Err("no update run is going in this repository".into()),
        }
    }

    /// What happened to this repository's run, or `None` if it has
    /// never had one in this process.
    pub fn state(&self, repo: &str) -> Option<RunState> {
        let runs = self.0.lock().unwrap_or_else(|e| e.into_inner());
        runs.get(repo).map(|e| e.state.clone())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn outcome() -> UpdateRunDone {
        UpdateRunDone {
            repo_path: "/repo".into(),
            url: None,
            branch: None,
            applied: 0,
            failed: 0,
            cancelled: false,
            error: None,
        }
    }

    #[test]
    fn a_second_run_in_the_same_repository_is_refused() {
        let runs = UpdateRuns::default();
        runs.start("/repo", 3).unwrap();
        assert!(runs.start("/repo", 3).is_err());
        // A different repository is unaffected: the guard is per-repo,
        // not global.
        assert!(runs.start("/other", 1).is_ok());
    }

    #[test]
    fn a_repository_can_run_again_once_the_first_has_ended() {
        let runs = UpdateRuns::default();
        runs.start("/repo", 3).unwrap();
        runs.finished("/repo", outcome());
        assert!(runs.start("/repo", 2).is_ok());
    }

    #[test]
    fn cancel_sets_the_flag_the_run_reads() {
        let runs = UpdateRuns::default();
        let stop = runs.start("/repo", 3).unwrap();
        assert!(!stop.load(Ordering::SeqCst));
        runs.cancel("/repo").unwrap();
        assert!(stop.load(Ordering::SeqCst));
    }

    #[test]
    fn cancelling_nothing_says_so() {
        let runs = UpdateRuns::default();
        assert!(runs.cancel("/repo").is_err());
        runs.start("/repo", 1).unwrap();
        runs.finished("/repo", outcome());
        // Finished, so there is nothing left to cancel.
        assert!(runs.cancel("/repo").is_err());
    }

    /// The resume read: a phone that slept through the whole run asks
    /// once and learns how it ended.
    #[test]
    fn the_outcome_survives_for_a_client_that_was_away() {
        let runs = UpdateRuns::default();
        runs.start("/repo", 5).unwrap();
        runs.progress("/repo", 2, 5);
        assert!(matches!(
            runs.state("/repo"),
            Some(RunState::Running { done: 2, total: 5 })
        ));
        runs.finished("/repo", outcome());
        assert!(matches!(runs.state("/repo"), Some(RunState::Done { .. })));
        // Still there on a second ask -- reading is not consuming.
        assert!(matches!(runs.state("/repo"), Some(RunState::Done { .. })));
    }

    #[test]
    fn a_repository_with_no_run_has_no_state() {
        assert!(UpdateRuns::default().state("/repo").is_none());
    }

    /// A progress frame that arrives after the terminal one must not
    /// turn a finished run back into a running one -- the phone would
    /// then wait forever for an outcome it had already been given.
    #[test]
    fn a_late_progress_frame_cannot_resurrect_a_finished_run() {
        let runs = UpdateRuns::default();
        runs.start("/repo", 5).unwrap();
        runs.finished("/repo", outcome());
        runs.progress("/repo", 4, 5);
        assert!(matches!(runs.state("/repo"), Some(RunState::Done { .. })));
    }
}
