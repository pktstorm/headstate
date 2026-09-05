import { act, cleanup, renderHook } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { mmss, useCountdown } from "./countdown";

beforeEach(() => {
  vi.useFakeTimers();
  vi.setSystemTime(new Date("2026-09-05T10:00:00Z"));
});
afterEach(() => {
  cleanup();
  vi.useRealTimers();
});

describe("useCountdown", () => {
  it("reads zero with no deadline", () => {
    const { result } = renderHook(() => useCountdown(null));
    expect(result.current).toBe(0);
  });

  it("ticks down once a second and stops at zero", () => {
    const deadline = Date.now() + 3000;
    const { result } = renderHook(() => useCountdown(deadline));
    expect(result.current).toBe(3);
    act(() => vi.advanceTimersByTime(1000));
    expect(result.current).toBe(2);
    act(() => vi.advanceTimersByTime(5000));
    expect(result.current).toBe(0);
  });

  // The Rust side expires on the wall clock. A count that paused with
  // a throttled webview would show time the token no longer has.
  it("follows the wall clock, not the number of ticks", () => {
    const deadline = Date.now() + 120_000;
    const { result } = renderHook(() => useCountdown(deadline));
    act(() => {
      vi.setSystemTime(Date.now() + 90_000);
      vi.advanceTimersByTime(1000);
    });
    expect(result.current).toBe(29);
  });
});

describe("mmss", () => {
  it("formats minutes and zero-padded seconds", () => {
    expect(mmss(120)).toBe("2:00");
    expect(mmss(119)).toBe("1:59");
    expect(mmss(5)).toBe("0:05");
    expect(mmss(0)).toBe("0:00");
  });
});
