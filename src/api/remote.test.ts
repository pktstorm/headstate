import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

const tauri = vi.hoisted(() => ({
  invoke: vi.fn<(cmd: string, args?: unknown) => Promise<unknown>>(() =>
    Promise.resolve(undefined),
  ),
  listen: vi.fn<(event: string, cb: unknown) => Promise<() => void>>(() =>
    Promise.resolve(() => {}),
  ),
}));
vi.mock("@tauri-apps/api/core", () => ({ invoke: tauri.invoke }));
vi.mock("@tauri-apps/api/event", () => ({ listen: tauri.listen }));

import { remote } from "./remote";

function setVisibility(state: DocumentVisibilityState) {
  Object.defineProperty(document, "visibilityState", { value: state, configurable: true });
  document.dispatchEvent(new Event("visibilitychange"));
}

beforeEach(() => {
  tauri.invoke.mockClear();
  tauri.listen.mockClear();
});

afterEach(() => {
  vi.restoreAllMocks();
});

describe("remote transport: commands", () => {
  it("forwards a desktop command through remote_call with an object of args", async () => {
    tauri.invoke.mockResolvedValueOnce([{ number: 1347 }]);
    await expect(remote.call("get_cached")).resolves.toEqual([{ number: 1347 }]);
    expect(tauri.invoke).toHaveBeenCalledWith("remote_call", { command: "get_cached", args: {} });

    await remote.call("get_history", { days: 14 });
    expect(tauri.invoke).toHaveBeenLastCalledWith("remote_call", {
      command: "get_history",
      args: { days: 14 },
    });
  });

  it("invokes the companion's own commands directly, arity preserved", async () => {
    tauri.invoke.mockResolvedValueOnce({
      state: "connected",
      desktop: "octocat's laptop",
      last_poll: null,
      protocol_version: 1,
      stale: false,
    });
    await expect(remote.call("connection_state")).resolves.toMatchObject({ state: "connected" });
    expect(tauri.invoke).toHaveBeenCalledWith("connection_state");
    expect(tauri.invoke.mock.calls[0]).toHaveLength(1);

    tauri.invoke.mockResolvedValueOnce("octocat's laptop");
    await expect(remote.call("pair_from_qr", { payload: "{}" })).resolves.toBe("octocat's laptop");
    expect(tauri.invoke).toHaveBeenLastCalledWith("pair_from_qr", { payload: "{}" });

    for (const name of ["unpair", "subscribe_events"]) {
      await remote.call(name);
      expect(tauri.invoke).toHaveBeenLastCalledWith(name);
    }
  });

  it("passes the companion's refusal through as the rejection", async () => {
    const refusal = "octocat's laptop is unreachable; actions are disabled until it is back";
    tauri.invoke.mockRejectedValueOnce(refusal);
    await expect(remote.call("act_on_pr", { id: "PR_1" })).rejects.toBe(refusal);
  });
});

describe("remote transport: events", () => {
  it("listens locally and opens the stream once, then again on each return to the foreground", async () => {
    const cb = () => {};
    const un = await remote.listen("prs-updated", cb);
    await remote.listen("poll-state", cb);
    expect(tauri.listen).toHaveBeenNthCalledWith(1, "prs-updated", cb);
    expect(tauri.listen).toHaveBeenNthCalledWith(2, "poll-state", cb);
    expect(typeof un).toBe("function");
    const subscribes = () =>
      tauri.invoke.mock.calls.filter(([cmd]) => cmd === "subscribe_events").length;
    expect(subscribes()).toBe(1);

    setVisibility("hidden");
    expect(subscribes()).toBe(1);
    setVisibility("visible");
    expect(subscribes()).toBe(2);
  });

  it("does not let a refused subscription surface", async () => {
    tauri.invoke.mockRejectedValueOnce("not paired with a desktop");
    setVisibility("visible");
    await Promise.resolve();
    expect(tauri.invoke).toHaveBeenCalledWith("subscribe_events");
  });
});
