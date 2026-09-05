import { beforeEach, describe, expect, it, vi } from "vitest";

const invoke = vi.hoisted(() => vi.fn(() => Promise.resolve("ok")));
const listen = vi.hoisted(() => vi.fn(() => Promise.resolve(() => {})));
vi.mock("@tauri-apps/api/core", () => ({ invoke }));
vi.mock("@tauri-apps/api/event", () => ({ listen }));

import { local } from "./local";

beforeEach(() => {
  invoke.mockClear();
  listen.mockClear();
});

/// The desktop transport must hand Tauri exactly what the wrappers used
/// to: a bare `invoke(name)` for a command without arguments, never
/// `invoke(name, undefined)`. Tests across the tree assert on that
/// one-argument shape (`toHaveBeenCalledWith("get_remote_enabled")`),
/// and they are the behaviour contract this seam promises not to move.
describe("local transport", () => {
  it("calls invoke with only the name when there are no args", async () => {
    await expect(local.call("get_cached")).resolves.toBe("ok");
    expect(invoke).toHaveBeenCalledTimes(1);
    expect(invoke.mock.calls[0]).toEqual(["get_cached"]);
  });

  it("calls invoke with name and args when there are args", async () => {
    await local.call("get_history", { days: 7 });
    expect(invoke.mock.calls[0]).toEqual(["get_history", { days: 7 }]);
  });

  it("subscribes through Tauri listen with the same name and callback", async () => {
    const cb = () => {};
    const un = await local.listen("prs-updated", cb);
    expect(listen.mock.calls[0]).toEqual(["prs-updated", cb]);
    expect(typeof un).toBe("function");
  });
});
