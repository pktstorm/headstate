import { useEffect, useRef, useState, type RefObject } from "react";

/// Pull down on a scrolled-to-top list to refresh it.
///
/// The phone has no other way to ask. The desktop has the `r` shortcut
/// and the tray's "Refresh now"; a phone user has neither, and the poll
/// loop runs on the DESKTOP -- so a companion showing stale rows had no
/// gesture to correct them. The pairing walkthrough instructs "pull to
/// refresh" at three separate steps (2.2, 6.1, 7.5) and the gesture did
/// not exist, which is what #639 was.
///
/// Deliberately hand-rolled rather than a library. It is three touch
/// handlers and a distance threshold; a dependency for that would be
/// more code to audit than the code it replaces, and it would have
/// opinions about the spinner this app does not want.
///
/// # Why the scroll position matters
///
/// The gesture only starts when the container is already at the top.
/// Otherwise a normal upward flick through a long list would arm it,
/// and the list would refresh whenever someone scrolled back to the
/// beginning. `scrollTop === 0` at TOUCH START, not during the move:
/// checking continuously would arm mid-flick the moment the top was
/// reached.

/// How far the finger travels before a release refreshes. Below this a
/// pull is a scroll that did not go anywhere, and refreshing on it
/// would fire on stray taps.
const THRESHOLD_PX = 64;

/// Where the indicator stops following the finger. Past this the pull
/// still counts, but the UI stops implying more travel achieves more.
const MAX_PULL_PX = 96;

export interface PullToRefresh {
  /// How far the finger has pulled, in pixels, clamped to
  /// `MAX_PULL_PX`. Zero when not pulling.
  distance: number;
  /// Whether releasing now would refresh.
  armed: boolean;
  /// Whether a refresh started by this gesture is still running.
  refreshing: boolean;
}

/// Wire the gesture to a scroll container.
///
/// `onRefresh` is awaited, so the indicator stays up until the refresh
/// actually finishes rather than for a fixed animation. A rejection is
/// swallowed here on purpose: the caller reports failures through its
/// own channel (the poll-error banner), and a gesture that also threw
/// would double-report.
///
/// Pass `enabled: false` to attach nothing at all -- on the desktop
/// build there is no reason to hold three listeners on the main scroll
/// container for a gesture a mouse cannot make.
export function usePullToRefresh(
  ref: RefObject<HTMLElement | null>,
  onRefresh: () => Promise<unknown>,
  enabled: boolean,
): PullToRefresh {
  const [distance, setDistance] = useState(0);
  const [refreshing, setRefreshing] = useState(false);
  // A ref, NOT a local in the effect. The effect re-runs whenever its
  // deps change, and a fresh closure would start with `busy = false`
  // while a refresh was still in flight -- so a second pull got
  // through and fired a second refresh. Caught by
  // "ignores a second gesture while a refresh is still running".
  const busyRef = useRef(false);

  useEffect(() => {
    const el = ref.current;
    if (!enabled || el === null) return;

    // Null means "not pulling". Set on touchstart only when the
    // container is at the top, which is what keeps a mid-list flick
    // from arming the gesture.
    let startY: number | null = null;
    let pulled = 0;

    const onStart = (e: TouchEvent) => {
      if (busyRef.current || el.scrollTop > 0) return;
      startY = e.touches[0]?.clientY ?? null;
      pulled = 0;
    };

    const onMove = (e: TouchEvent) => {
      // Re-checked here, not only on touchstart: a gesture that began
      // BEFORE the refresh started is still live in `startY`.
      if (busyRef.current || startY === null) return;
      const y = e.touches[0]?.clientY;
      if (y === undefined) return;
      const delta = y - startY;
      if (delta <= 0) {
        // Pulling up: this is an ordinary scroll after all. Stand
        // down rather than tracking a negative distance.
        startY = null;
        pulled = 0;
        setDistance(0);
        return;
      }
      // Damped, not one-to-one: a rubber-band that moves slower than
      // the finger reads as resistance, and reaching the threshold
      // stays a deliberate act rather than an accident of a fast
      // swipe.
      pulled = Math.min(delta / 2, MAX_PULL_PX);
      setDistance(pulled);
    };

    const onEnd = () => {
      const shouldRefresh = !busyRef.current && startY !== null && pulled >= THRESHOLD_PX;
      startY = null;
      pulled = 0;
      if (!shouldRefresh) {
        setDistance(0);
        return;
      }
      busyRef.current = true;
      setRefreshing(true);
      // Held at the threshold while the refresh runs, so the indicator
      // does not snap back and leave the spinner floating unanchored.
      setDistance(THRESHOLD_PX);
      void onRefresh().catch(() => {}).finally(() => {
        busyRef.current = false;
        setRefreshing(false);
        setDistance(0);
      });
    };

    // `passive: true` on move: this never calls `preventDefault`. The
    // browser's own overscroll is left alone rather than fought, which
    // keeps the gesture from feeling like it is competing with the
    // webview's bounce -- and a non-passive move listener on a scroll
    // container is a well-known way to make scrolling janky.
    el.addEventListener("touchstart", onStart, { passive: true });
    el.addEventListener("touchmove", onMove, { passive: true });
    el.addEventListener("touchend", onEnd);
    el.addEventListener("touchcancel", onEnd);
    return () => {
      el.removeEventListener("touchstart", onStart);
      el.removeEventListener("touchmove", onMove);
      el.removeEventListener("touchend", onEnd);
      el.removeEventListener("touchcancel", onEnd);
    };
  }, [ref, onRefresh, enabled]);

  return { distance, armed: distance >= THRESHOLD_PX, refreshing };
}

export { THRESHOLD_PX as PULL_THRESHOLD_PX };
