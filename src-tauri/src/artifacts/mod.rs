//! Regenerable build output: `target/`, `node_modules/`, `.terraform/`,
//! and gitignored `dist`/`build` directories.
//!
//! A separate concern from `worktrees`, despite both walking the same
//! directories, because the two answer opposite questions. A worktree may
//! hold work that exists nowhere else, so its safety check asks "would
//! removing this lose something irreplaceable?" -- and the answer takes
//! git plumbing, an upstream probe, and a merge check. Build output is
//! regenerable by definition, so the only questions here are "is a tool's
//! output really what this is?" and "is something writing to it right
//! now?".
//!
//! That difference is why this is not an extension of the worktree view.
//! Measured on the machine that prompted it: only 0.28 GB of Rust target
//! space sat inside worktrees, against 108 GB beside main checkouts. The
//! worktree feature structurally could not reach 99.7% of the largest
//! thing on the disk.
//!
//! Nothing here talks to GitHub.

pub mod model;
pub mod scan;

pub use model::{Artifact, ArtifactKind};
pub use scan::scan;

use std::path::Path;
use std::time::SystemTime;

/// Bytes on disk, and how recently anything under it was written.
///
/// One walk for both, because the expensive part is the traversal and
/// asking two questions of the same `metadata()` call is free.
///
/// Skips symlinks, so a link into another tree is neither counted twice
/// nor mistaken for local content -- the same rule `worktrees::dir_size`
/// applies, for the same reason.
///
/// Unreadable entries are skipped rather than failing the measurement: a
/// permission error on one file should not turn a real size into
/// "unknown".
pub fn measure(path: &Path) -> (u64, Option<u64>) {
    let mut total = 0u64;
    let mut newest: Option<SystemTime> = None;
    let mut stack = vec![path.to_path_buf()];

    while let Some(dir) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for e in entries.flatten() {
            let Ok(meta) = e.metadata() else { continue };
            if meta.is_symlink() {
                continue;
            }
            if let Ok(m) = meta.modified() {
                if newest.is_none_or(|n| m > n) {
                    newest = Some(m);
                }
            }
            if meta.is_dir() {
                stack.push(e.path());
            } else {
                total += meta.len();
            }
        }
    }

    let age = newest.and_then(|n| n.elapsed().ok()).map(|d| d.as_secs());
    (total, age)
}
