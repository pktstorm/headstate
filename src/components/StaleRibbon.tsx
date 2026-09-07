import { isStale, useConnectionState } from "@/api/connection";
import { relativeTime } from "@/lib/time";

/// Says, on the data itself, that what is on screen is a cached copy.
///
/// The companion serves `get_cached` from its stored snapshot whenever
/// the desktop is unreachable, so the phone went on rendering a full,
/// confident pull-request list -- correct-looking rows, CI badges, merge
/// states -- that could be hours old. `StatusBar` made it worse: its dot
/// is computed from poll-loop events that simply stop arriving on a dead
/// stream, so it sat on a green "Up to date" beside stale rows, which is
/// precisely the failure its own doc comment says it was rewritten to
/// eliminate.
///
/// The connection banner did say "unreachable", but it is one line at
/// the top of the window and `ConnectionBanner` was the ONLY component
/// in the frontend reading the connection state at all. A marker that
/// sits above the list is easy to scroll past; this one is attached to
/// the content, which is the thing being doubted.
///
/// Renders nothing on the desktop, and nothing when the desktop is
/// reachable -- `isStale` is false for `local` by construction.
export function StaleRibbon() {
  const state = useConnectionState();
  if (!isStale(state)) return null;
  // `isStale` already excluded every variant without these fields.
  const { desktop, lastPoll } = state as { desktop: string; lastPoll: string | null };
  return (
    <div
      role="status"
      className="flex shrink-0 items-center gap-2 border-b border-[#d29922]/30 bg-[#d29922]/10 px-4 py-1.5 text-xs text-[#d29922]"
    >
      <span className="min-w-0 flex-1">
        Showing a saved copy — {desktop} is not reachable
        {lastPoll === null ? "" : `, last updated ${relativeTime(lastPoll)}`}. Actions are
        paused until it is back.
      </span>
    </div>
  );
}
