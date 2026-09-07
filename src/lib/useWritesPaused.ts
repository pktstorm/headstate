import { isStale, useConnectionState } from "@/api/connection";

/// Why writes are paused, or null when they are not.
///
/// The companion refuses write and destructive commands whenever it
/// cannot drive the paired desktop -- unreachable, revoked, or a desktop
/// below the required protocol -- and answers with a message naming the
/// desktop and the reason. That refusal was the FIRST thing the user
/// heard about it: the buttons stayed live, the tap went through, and a
/// toast came back some seconds later.
///
/// This is the same condition, asked before the tap rather than after,
/// so the control can carry its reason like every other unavailable
/// action in the app (see `unavailable` in `PrKebab.tsx`, which greys out
/// with a tooltip rather than hiding).
///
/// It is deliberately NOT a second copy of the rule. Rust decides, in
/// `connection.rs`, and reports it as `stale` on every connection
/// report; this reads that flag. A client-side re-derivation would drift
/// from the desktop's answer, and drifting towards "allowed" is exactly
/// the direction that hurts.
///
/// Returns null on the desktop, always: `isStale` is false for `local`,
/// so nothing on the desktop is ever disabled by this.
export function useWritesPaused(): string | null {
  const state = useConnectionState();
  if (!isStale(state)) return null;
  const { desktop } = state as { desktop: string };
  return `${desktop} is not reachable`;
}
