import { act, renderHook } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import { PULL_THRESHOLD_PX, usePullToRefresh } from "./usePullToRefresh";

/// The gesture the pairing walkthrough instructs at steps 2.2, 6.1 and
/// 7.5, and which did not exist (#639). The phone has no other way to
/// ask for fresh data: the `r` shortcut needs a keyboard, the tray is a
/// desktop affordance, and the poll loop runs on the desktop.

/// A stand-in scroll container. `scrollTop` is writable so a test can
/// place the list mid-scroll, which is what decides whether the gesture
/// arms at all.
function container(scrollTop = 0) {
  const el = document.createElement("div");
  Object.defineProperty(el, "scrollTop", { value: scrollTop, writable: true });
  document.body.appendChild(el);
  return el;
}

function touch(el: HTMLElement, type: string, clientY: number) {
  const e = new Event(type, { bubbles: true }) as TouchEvent & { touches: unknown };
  Object.defineProperty(e, "touches", {
    value: type === "touchend" || type === "touchcancel" ? [] : [{ clientY }],
  });
  el.dispatchEvent(e);
}

/// Pull far enough to arm, in one move. The hook damps travel by half,
/// so the finger has to move twice the threshold.
function pullPast(el: HTMLElement, from = 0) {
  touch(el, "touchstart", from);
  touch(el, "touchmove", from + PULL_THRESHOLD_PX * 2 + 20);
}

afterEach(() => {
  document.body.replaceChildren();
});

describe("usePullToRefresh", () => {
  it("refreshes when pulled past the threshold and released", async () => {
    const el = container();
    const ref = { current: el };
    const onRefresh = vi.fn().mockResolvedValue(undefined);
    const { result } = renderHook(() => usePullToRefresh(ref, onRefresh, true));

    act(() => pullPast(el));
    expect(result.current.armed).toBe(true);

    await act(async () => {
      touch(el, "touchend", 0);
    });
    expect(onRefresh).toHaveBeenCalledTimes(1);
    // Back to rest once the refresh settles, so the spinner does not
    // strand itself on screen.
    expect(result.current.refreshing).toBe(false);
    expect(result.current.distance).toBe(0);
  });

  it("does nothing for a pull that never reaches the threshold", () => {
    const el = container();
    const onRefresh = vi.fn().mockResolvedValue(undefined);
    renderHook(() => usePullToRefresh({ current: el }, onRefresh, true));

    act(() => {
      touch(el, "touchstart", 0);
      // Half the threshold of finger travel is a quarter of it after
      // damping: a stray tap, not a request.
      touch(el, "touchmove", PULL_THRESHOLD_PX / 2);
      touch(el, "touchend", 0);
    });
    expect(onRefresh).not.toHaveBeenCalled();
  });

  it("does not arm when the list is scrolled away from the top", () => {
    // The rule that keeps an ordinary upward flick through a long list
    // from refreshing it the moment the top comes into view.
    const el = container(400);
    const onRefresh = vi.fn().mockResolvedValue(undefined);
    const { result } = renderHook(() => usePullToRefresh({ current: el }, onRefresh, true));

    act(() => {
      pullPast(el);
      touch(el, "touchend", 0);
    });
    expect(onRefresh).not.toHaveBeenCalled();
    expect(result.current.distance).toBe(0);
  });

  it("stands down when the finger reverses into a scroll", () => {
    const el = container();
    const onRefresh = vi.fn().mockResolvedValue(undefined);
    const { result } = renderHook(() => usePullToRefresh({ current: el }, onRefresh, true));

    act(() => {
      touch(el, "touchstart", 100);
      touch(el, "touchmove", 40); // upward
      touch(el, "touchend", 0);
    });
    expect(onRefresh).not.toHaveBeenCalled();
    expect(result.current.distance).toBe(0);
  });

  it("attaches nothing when disabled", () => {
    // The desktop build. No reason to hold three listeners on the main
    // scroll container for a gesture a mouse cannot make.
    const el = container();
    const onRefresh = vi.fn().mockResolvedValue(undefined);
    renderHook(() => usePullToRefresh({ current: el }, onRefresh, false));

    act(() => {
      pullPast(el);
      touch(el, "touchend", 0);
    });
    expect(onRefresh).not.toHaveBeenCalled();
  });

  it("ignores a second gesture while a refresh is still running", async () => {
    // Otherwise a fast double-pull fires two refreshes and the
    // indicator's state fights itself.
    const el = container();
    let release: () => void = () => {};
    const onRefresh = vi.fn(
      () =>
        new Promise<void>((r) => {
          release = r;
        }),
    );
    const { result } = renderHook(() => usePullToRefresh({ current: el }, onRefresh, true));

    act(() => {
      pullPast(el);
      touch(el, "touchend", 0);
    });
    expect(result.current.refreshing).toBe(true);

    act(() => {
      pullPast(el);
      touch(el, "touchend", 0);
    });
    expect(onRefresh).toHaveBeenCalledTimes(1);

    await act(async () => {
      release();
    });
    expect(result.current.refreshing).toBe(false);
  });

  it("recovers when the refresh fails", async () => {
    // The failure is reported through the poll-error banner, not here,
    // so the gesture's only job is not to strand the spinner.
    const el = container();
    const onRefresh = vi.fn().mockRejectedValue(new Error("offline"));
    const { result } = renderHook(() => usePullToRefresh({ current: el }, onRefresh, true));

    await act(async () => {
      pullPast(el);
      touch(el, "touchend", 0);
    });
    expect(result.current.refreshing).toBe(false);
    expect(result.current.distance).toBe(0);
  });

  it("treats a cancelled touch as a release", async () => {
    // iOS cancels a touch when a system gesture takes over. Without
    // this the indicator would stay pulled down forever.
    const el = container();
    const onRefresh = vi.fn().mockResolvedValue(undefined);
    const { result } = renderHook(() => usePullToRefresh({ current: el }, onRefresh, true));

    await act(async () => {
      pullPast(el);
      touch(el, "touchcancel", 0);
    });
    expect(result.current.distance).toBe(0);
  });
});
