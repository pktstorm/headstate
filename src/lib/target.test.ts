import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

// `IS_MOBILE_BUILD` is read at module load, so each case needs a fresh
// import after stubbing -- the same shape `transport.test.ts` uses for
// its own load-time read of `VITE_TARGET`.
async function loadTarget(value: string) {
  vi.stubEnv("VITE_TARGET", value);
  vi.resetModules();
  return await import("./target");
}

beforeEach(() => {
  vi.resetModules();
});

afterEach(() => {
  vi.unstubAllEnvs();
});

describe("IS_MOBILE_BUILD", () => {
  it("is true on the mobile build", async () => {
    const { IS_MOBILE_BUILD, IS_DESKTOP_BUILD } = await loadTarget("mobile");
    expect(IS_MOBILE_BUILD).toBe(true);
    expect(IS_DESKTOP_BUILD).toBe(false);
  });

  it("is false on the desktop build", async () => {
    const { IS_MOBILE_BUILD, IS_DESKTOP_BUILD } = await loadTarget("desktop");
    expect(IS_MOBILE_BUILD).toBe(false);
    expect(IS_DESKTOP_BUILD).toBe(true);
  });

  it("does not follow the viewport", async () => {
    // The whole point of the constant. A desktop build in a narrow
    // window still has `gh`, a filesystem and an updater, so anything
    // gated on capability must stay on the desktop branch -- which is
    // exactly what `useIsMobile()` cannot express.
    const { stubViewport } = await import("@/test-utils");
    stubViewport(390);
    const { IS_MOBILE_BUILD } = await loadTarget("desktop");
    expect(IS_MOBILE_BUILD).toBe(false);
    stubViewport(null);
  });

  it("treats an unset target as the desktop", async () => {
    // `vite.config.ts` defaults the define to "desktop"; a bundle that
    // somehow lost it must fail safe towards the app that has the
    // capabilities, not towards the one that does not.
    const { IS_MOBILE_BUILD } = await loadTarget("");
    expect(IS_MOBILE_BUILD).toBe(false);
  });
});
