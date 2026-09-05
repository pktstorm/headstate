import { useEffect, useState } from "react";

/// Whole seconds until `deadlineMs`, never below zero, re-rendered once
/// a second while there is time left. Null means "no deadline" and
/// reads as zero.
///
/// Driven by `Date.now()` rather than by counting ticks, so a webview
/// that was throttled in the background (timers coalesced or paused)
/// shows the true remaining time when it comes back rather than a
/// count that stopped with it -- the Rust side expires on the clock,
/// not on how many times this fired.
///
/// The clock is sampled at mount and on each tick, never during render
/// (React's purity rule). A component whose deadline can change should
/// carry the deadline in its `key`, so a new one mounts fresh instead
/// of reading up to a second of stale time until the next tick.
export function useCountdown(deadlineMs: number | null): number {
  const [now, setNow] = useState(() => Date.now());
  const secs = deadlineMs === null ? 0 : Math.max(0, Math.ceil((deadlineMs - now) / 1000));
  const running = deadlineMs !== null && secs > 0;

  useEffect(() => {
    if (!running) return;
    const id = setInterval(() => setNow(Date.now()), 1000);
    return () => clearInterval(id);
  }, [running]);

  return secs;
}

/// `m:ss`, as a countdown is read.
export function mmss(secs: number): string {
  const m = Math.floor(secs / 60);
  const s = secs % 60;
  return `${m}:${s.toString().padStart(2, "0")}`;
}
