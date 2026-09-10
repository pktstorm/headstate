import { Bot, MoreHorizontal, RotateCcw, Trash2 } from "lucide-react";
import { useEffect, useRef, useState } from "react";
import type { Worktree } from "../types/pr";
import { forceWarning, isSafe, safetyReason } from "../lib/worktrees";

/// Per-row actions for a worktree.
///
/// The action cell holds ONE button by design — the row is dense, and a
/// fixed width keeps a label swap from reflowing every column (#393).
/// So the further actions live behind a kebab rather than beside the
/// first, the same shape `PrKebab` uses.
///
/// It began as the way BACK from "I read the assessment", which was a
/// one-way door: Claudify was replaced by "Remove anyway…", the mark
/// persisted across restarts, and only a moved branch cleared it. A
/// single exploratory click permanently removed the only way to copy
/// that worktree's prompt.
///
/// #770 gave it the more important job. Removal past the safety gate
/// used to be reachable ONLY from the Claudify toast's "I read the
/// assessment" button, and a toast is for something you can ignore — it
/// leaves on a timer or on a stray click, and nothing about it says it
/// is the only route to the thing you just asked for. The more careful
/// the user was being, the more likely they lost it, and the only way
/// back was to re-run Claudify and spend another agent invocation to
/// recover a button. So the menu is now on EVERY row, and it carries
/// removal on a surface that does not disappear.
export function WorktreeKebab({
  worktree,
  assessed = false,
  onClaudify,
  onForget,
  onRemove,
  onForce,
}: {
  worktree: Worktree;
  /// This worktree has been handed to Claude Code and the branch has not
  /// moved since. Decides whether the Claudify/Forget pair is offered —
  /// NOT whether removal is, which #770 deliberately decoupled from it.
  assessed?: boolean;
  onClaudify: (wt: Worktree) => void;
  onForget: (wt: Worktree) => void;
  /// The plain, confirmed removal. Only ever called for a row the gate
  /// already considers safe.
  onRemove: (wt: Worktree) => void;
  /// The override, behind the same confirmation the "Remove anyway…"
  /// button opens. It relaxes Headstate's gate, not git's.
  onForce: (wt: Worktree) => void;
}) {
  const [open, setOpen] = useState(false);
  const ref = useRef<HTMLDivElement>(null);

  useEffect(() => {
    if (!open) return;
    const onKey = (e: KeyboardEvent) => e.key === "Escape" && setOpen(false);
    const onClick = (e: MouseEvent) => {
      if (ref.current && !ref.current.contains(e.target as Node)) setOpen(false);
    };
    window.addEventListener("keydown", onKey);
    window.addEventListener("mousedown", onClick);
    return () => {
      window.removeEventListener("keydown", onKey);
      window.removeEventListener("mousedown", onClick);
    };
  }, [open]);

  const item =
    "flex w-full items-center gap-2 rounded px-2 py-1.5 text-left text-xs text-[#e6edf3] hover:bg-[#21262d]";
  // Red, like every other removal affordance on this page. A menu item
  // that deletes a directory must not look like one that copies a
  // string.
  const destructive =
    "flex w-full items-start gap-2 rounded px-2 py-1.5 text-left text-xs text-[#f85149] hover:bg-[#f85149]/10";

  // The gate, read here exactly as the primary button reads it (#770).
  //
  // `isSafe` covers `safe` AND `merged_upstream_deleted`: both mean the
  // work is on the default branch and the tree is clean, so both take
  // the plain confirmed path. Everything else goes through `onForce`,
  // which opens the confirmation naming the specific loss.
  //
  // The menu never HIDES removal — that is the entire point of the
  // issue. What varies is which route it takes and what it says first.
  const safe = isSafe(worktree.safety);
  // Locked is offered, and says up front that forcing will not help.
  //
  // `remove_worktree_forced` relaxes Headstate's gate but still calls
  // git WITHOUT `--force`, and git refuses a locked tree on its own
  // account. Without this line the user confirms a destructive-sounding
  // dialog and gets an error. `forceWarning` already owns that
  // sentence for #753's confirmation, so it is reused rather than
  // reworded: two copies of a warning about an unrecoverable action are
  // two chances to drift.
  const locked = worktree.safety.kind === "locked";
  // Two rows are left out, and only two.
  //
  // An ORPHAN has no repository for git to run in, so it is removed by
  // an entirely different call -- the Rust side deletes the directory
  // after re-checking. The row's own Delete button already offers that,
  // and a second, wrong route from the menu would be worse than none.
  //
  // The MAIN CHECKOUT is never a removal candidate at all. It is the
  // one row on the page where offering this would be a serious bug
  // rather than a widened gate.
  //
  // PRUNABLE deliberately stays IN. Its directory is already gone, but
  // the stale registration remains and removing it is real work that
  // loses nothing -- `forceWarning` has copy saying exactly that, so
  // the confirmation is already honest about it.
  const removable = worktree.safety.kind !== "orphaned" && !worktree.is_main;

  return (
    <div ref={ref} className="relative shrink-0" onClick={(e) => e.stopPropagation()}>
      <button
        type="button"
        // Falls back to the path for a DETACHED worktree, which has no
        // branch: the label was "More actions for " with nothing after
        // it, so a screen reader user could not tell two such rows
        // apart -- and this menu now removes directories.
        aria-label={`More actions for ${worktree.branch || worktree.path}`}
        aria-expanded={open}
        aria-haspopup="menu"
        onClick={() => setOpen((o) => !o)}
        className="rounded p-1 text-[#8b949e] hover:bg-[#21262d] hover:text-[#e6edf3]"
      >
        <MoreHorizontal className="h-4 w-4" aria-hidden="true" />
      </button>

      {open ? (
        <div
          role="menu"
          className="absolute right-0 top-full z-20 mt-1 w-56 rounded border border-[#30363d] bg-[#161b22] p-1 shadow-lg"
        >
          {/* Only once an assessment has been marked. Before that the
              row's own button already IS Claudify, and a menu holding a
              duplicate of the button beside it would be noise. */}
          {assessed ? (
            <>
              {/* The whole point: needing the prompt again is normal.
                  The terminal was closed, the paste was lost, the
                  assessment wants rerunning. */}
              <button
                type="button"
                role="menuitem"
                className={item}
                onClick={() => {
                  setOpen(false);
                  onClaudify(worktree);
                }}
              >
                <Bot className="h-3 w-3" aria-hidden="true" />
                Copy the Claudify command
              </button>
              {/* Re-LOCKS the force-removal path, so it is the safe
                  direction and needs no confirmation. */}
              <button
                type="button"
                role="menuitem"
                className={item}
                onClick={() => {
                  setOpen(false);
                  onForget(worktree);
                }}
              >
                <RotateCcw className="h-3 w-3" aria-hidden="true" />
                Forget the assessment
              </button>
              <div className="my-1 border-t border-[#30363d]" />
            </>
          ) : null}

          {/* Removal, on a surface that does not disappear (#770).

              The route differs by verdict and the wording says which:
              a safe row goes to the ordinary confirmation, everything
              else to the one that names the specific loss. Neither is
              silent, and neither is hidden -- the menu's job is to make
              a considered human judgement reachable, not to re-litigate
              the gate that already refused. */}
          {removable ? (
            <button
              type="button"
              role="menuitem"
              className={destructive}
              onClick={() => {
                setOpen(false);
                if (safe) onRemove(worktree);
                else onForce(worktree);
              }}
            >
              <Trash2 className="mt-0.5 h-3 w-3 shrink-0" aria-hidden="true" />
              <span className="min-w-0">
                Remove worktree
                {/* The second line is the whole reason a locked row is
                    offered at all rather than greyed out: the user CAN
                    proceed, and needs to know before clicking that the
                    obstacle is git's and not Headstate's. Everything
                    else gets its verdict restated, so the menu item is
                    never a bare "Remove" divorced from the reason the
                    row was refused in the first place. */}
                {safe ? null : (
                  <span className="mt-0.5 block text-[#8b949e]">
                    {locked ? forceWarning(worktree.safety) : safetyReason(worktree.safety)}
                  </span>
                )}
              </span>
            </button>
          ) : null}
        </div>
      ) : null}
    </div>
  );
}
