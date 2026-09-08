import { Loader2, ArrowDown } from "lucide-react";
import type { PullToRefresh } from "@/lib/usePullToRefresh";

/// The spinner a pull-to-refresh gesture pulls into view.
///
/// Absolutely positioned and translated by the pull distance rather
/// than inserted into the flow: growing a real element would reflow the
/// list under the user's finger on every touchmove, which is both
/// expensive and visibly wrong.
///
/// Renders nothing at rest, so the desktop -- where the gesture is not
/// even wired up -- pays nothing for it.
export function PullIndicator({ state }: { state: PullToRefresh }) {
  const { distance, armed, refreshing } = state;
  if (distance === 0 && !refreshing) return null;
  return (
    <div
      aria-hidden="true"
      className="pointer-events-none absolute inset-x-0 top-0 z-10 flex justify-center"
      style={{ transform: `translateY(${distance - 28}px)` }}
    >
      <div className="flex h-7 w-7 items-center justify-center rounded-full border border-[#30363d] bg-[#161b22] text-[#8b949e] shadow">
        {refreshing ? (
          <Loader2 className="h-3.5 w-3.5 animate-spin" />
        ) : (
          <ArrowDown
            className={`h-3.5 w-3.5 transition-transform ${armed ? "rotate-180 text-[#58a6ff]" : ""}`}
          />
        )}
      </div>
    </div>
  );
}
