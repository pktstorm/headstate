import { clearMocks, mockIPC } from "@tauri-apps/api/mocks";
import { afterEach, describe, expect, it } from "vitest";
import { getRemoteEnabled, setRemoteEnabled } from "./tauri";

afterEach(() => {
  clearMocks();
});

/// The two commands behind **Allow phone connections**. Separate from
/// `tauri.test.ts` so the transport-seam rewrite of `tauri.ts` (#505)
/// rebases cleanly around it.
describe("phone connection wrappers", () => {
  it("getRemoteEnabled invokes get_remote_enabled", async () => {
    mockIPC((cmd) => (cmd === "get_remote_enabled" ? true : undefined));
    await expect(getRemoteEnabled()).resolves.toBe(true);
  });

  it("setRemoteEnabled invokes set_remote_enabled with the flag", async () => {
    let seen: unknown;
    mockIPC((cmd, args) => {
      if (cmd === "set_remote_enabled") seen = args;
      return undefined;
    });
    await setRemoteEnabled(true);
    expect(seen).toEqual({ enabled: true });
  });

  it("propagates a refused start as a rejected promise", async () => {
    mockIPC((cmd) => {
      if (cmd === "set_remote_enabled") throw "could not listen on 0.0.0.0:41919: in use";
      return undefined;
    });
    await expect(setRemoteEnabled(true)).rejects.toBe(
      "could not listen on 0.0.0.0:41919: in use",
    );
  });
});
