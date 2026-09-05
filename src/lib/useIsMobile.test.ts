import { act, renderHook } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import { stubViewport } from "@/test-utils";
import { MOBILE_BREAKPOINT, useIsMobile } from "./useIsMobile";

afterEach(() => {
  vi.unstubAllEnvs();
  stubViewport(null);
});

describe("useIsMobile", () => {
  it("is false when the environment has no matchMedia at all", () => {
    // jsdom ships none, which is also the SSR shape: the hook must not
    // throw on a window that cannot answer the question.
    stubViewport(null);
    const { result } = renderHook(() => useIsMobile());
    expect(result.current).toBe(false);
  });

  it("is false at desktop widths", () => {
    stubViewport(1400);
    const { result } = renderHook(() => useIsMobile());
    expect(result.current).toBe(false);
  });

  it("is true below the breakpoint", () => {
    stubViewport(390);
    const { result } = renderHook(() => useIsMobile());
    expect(result.current).toBe(true);
  });

  it("treats the breakpoint itself as desktop", () => {
    stubViewport(MOBILE_BREAKPOINT);
    expect(renderHook(() => useIsMobile()).result.current).toBe(false);
    stubViewport(MOBILE_BREAKPOINT - 1);
    expect(renderHook(() => useIsMobile()).result.current).toBe(true);
  });

  it("follows the viewport as it resizes", () => {
    const viewport = stubViewport(1400);
    const { result } = renderHook(() => useIsMobile());
    expect(result.current).toBe(false);
    act(() => viewport.resize(390));
    expect(result.current).toBe(true);
    act(() => viewport.resize(1400));
    expect(result.current).toBe(false);
  });

  it("is true on the mobile build whatever the viewport says", () => {
    vi.stubEnv("VITE_TARGET", "mobile");
    stubViewport(1400);
    const { result } = renderHook(() => useIsMobile());
    expect(result.current).toBe(true);
  });

  it("still honours a narrow viewport on the desktop build", () => {
    // The desktop window enforces 1000px, so this only ever happens in
    // a browser during development -- and a layout that responds there
    // is what lets the phone pass be checked without a phone.
    vi.stubEnv("VITE_TARGET", "desktop");
    stubViewport(390);
    const { result } = renderHook(() => useIsMobile());
    expect(result.current).toBe(true);
  });
});
