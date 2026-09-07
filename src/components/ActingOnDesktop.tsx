import { useConnectionState } from "@/api/connection";

/// Names the machine a destructive action will actually touch.
///
/// On the desktop there is no ambiguity -- the files being deleted are
/// on the computer the user is looking at -- so this renders nothing
/// there, and every existing confirmation reads exactly as it did.
///
/// On the phone there is nothing BUT ambiguity. Removing worktrees,
/// Docker images, artifacts and venvs from a phone is the companion's
/// headline feature, and those commands are `Class::Destructive` and
/// allowlisted, so they genuinely run -- on the desktop. The dialogs
/// were written for the desktop and say things like "Each one has been
/// replaced by a newer build" with no mention of a machine at all. A
/// confirmation that does not name what it is about to delete from is
/// the wrong confirmation to show on a second device.
///
/// Deliberately a separate line rather than a rewrite of each dialog's
/// sentence: the sentences are carefully worded for what is being
/// removed, and this adds where without disturbing what.
export function ActingOnDesktop() {
  const state = useConnectionState();
  if (state.kind === "local" || state.kind === "unknown" || state.kind === "unpaired") {
    return null;
  }
  return (
    <p className="mt-2 flex items-center gap-2 rounded border border-[#30363d] bg-[#161b22] px-3 py-2 text-sm text-[#8b949e]">
      <span aria-hidden="true">💻</span>
      <span>
        This runs on <span className="text-[#e6edf3]">{state.desktop}</span>, not on this
        phone.
      </span>
    </p>
  );
}
