import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { renderHook, waitFor } from "@testing-library/react";
import { clearMocks, mockIPC } from "@tauri-apps/api/mocks";
import type { ReactNode } from "react";
import { afterEach, describe, expect, it, vi } from "vitest";
import { isStale } from "./connection";

function wrapper({ children }: { children: ReactNode }) {
  const client = new QueryClient({ defaultOptions: { queries: { retry: false } } });
  return <QueryClientProvider client={client}>{children}</QueryClientProvider>;
}

/// The hook is chosen at module load from the build target, so each
/// test sets the target and then imports a fresh copy of the module.
async function load(target: "desktop" | "mobile") {
  vi.stubEnv("VITE_TARGET", target);
  vi.resetModules();
  const { useConnectionState } = await import("./connection");
  return useConnectionState;
}

afterEach(() => {
  clearMocks();
  vi.unstubAllEnvs();
});

describe("useConnectionState", () => {
  it("is local on the desktop build and never asks Rust", async () => {
    const useConnectionState = await load("desktop");
    const calls: string[] = [];
    mockIPC((cmd) => {
      calls.push(cmd);
      return undefined;
    });
    const { result } = renderHook(() => useConnectionState(), { wrapper });
    expect(result.current).toEqual({ kind: "local" });
    // Give a stray query a tick to fire, then confirm none did.
    await new Promise((r) => setTimeout(r, 0));
    expect(calls).toEqual([]);
  });

  it("needs no QueryClient on the desktop", async () => {
    // The banner mounts in every window; the desktop must not pay for
    // a query it never runs, and a test that renders App without a
    // provider must keep passing.
    const useConnectionState = await load("desktop");
    const { result } = renderHook(() => useConnectionState());
    expect(result.current).toEqual({ kind: "local" });
  });

  it("is unknown on the mobile build while the command is missing", async () => {
    const useConnectionState = await load("mobile");
    // #514 has not landed: the command is not registered, and Tauri
    // rejects the call. The hook must report that rather than throw.
    mockIPC(() => {
      throw new Error("command connection_state not found");
    });
    const { result } = renderHook(() => useConnectionState(), { wrapper });
    expect(result.current).toEqual({ kind: "unknown" });
    await new Promise((r) => setTimeout(r, 0));
    expect(result.current).toEqual({ kind: "unknown" });
  });

  it("maps a connected report to the banner's shape", async () => {
    const useConnectionState = await load("mobile");
    mockIPC((cmd) =>
      cmd === "connection_state"
        ? {
            state: "connected",
            desktop: "octocat's laptop",
            last_poll: "2026-09-04T10:00:00Z",
            protocol_version: 1,
          }
        : undefined,
    );
    const { result } = renderHook(() => useConnectionState(), { wrapper });
    await waitFor(() =>
      expect(result.current).toEqual({
        kind: "connected",
        desktop: "octocat's laptop",
        lastPoll: "2026-09-04T10:00:00Z",
        protocolVersion: 1,
        // Absent in the report: a connected desktop is assumed
        // driveable, which is what the field's default has to be for a
        // companion that predates it.
        stale: false,
      }),
    );
  });

  it("reads a missing protocol version as unknown, not as zero", async () => {
    // A report from before the field existed must not read as a
    // desktop on protocol 0, which the banner would tell the user to
    // update.
    const useConnectionState = await load("mobile");
    mockIPC(() => ({ state: "connected", desktop: "octocat's laptop", last_poll: null }));
    const { result } = renderHook(() => useConnectionState(), { wrapper });
    await waitFor(() => expect(result.current.kind).toBe("connected"));
    expect(result.current).toEqual({
      kind: "connected",
      desktop: "octocat's laptop",
      lastPoll: null,
      protocolVersion: null,
      stale: false,
    });
  });

  it("carries no protocol version on the states that cannot issue commands", async () => {
    const useConnectionState = await load("mobile");
    mockIPC(() => ({
      state: "connecting",
      desktop: "octocat's laptop",
      last_poll: null,
      protocol_version: 1,
    }));
    const { result } = renderHook(() => useConnectionState(), { wrapper });
    await waitFor(() => expect(result.current.kind).toBe("connecting"));
    expect(result.current).toEqual({
      kind: "connecting",
      desktop: "octocat's laptop",
      lastPoll: null,
      // Absent, and not connected: the phone is not driving anything
      // yet, so the data on screen cannot be called live.
      stale: true,
    });
  });

  it("maps unpaired to a state with no desktop", async () => {
    const useConnectionState = await load("mobile");
    mockIPC(() => ({ state: "unpaired", desktop: null, last_poll: null }));
    const { result } = renderHook(() => useConnectionState(), { wrapper });
    await waitFor(() => expect(result.current).toEqual({ kind: "unpaired" }));
  });

  it("never renders a null desktop name", async () => {
    const useConnectionState = await load("mobile");
    mockIPC(() => ({ state: "unreachable", desktop: null, last_poll: null }));
    const { result } = renderHook(() => useConnectionState(), { wrapper });
    await waitFor(() => expect(result.current.kind).toBe("unreachable"));
    expect(result.current).toEqual({
      kind: "unreachable",
      desktop: "Desktop",
      lastPoll: null,
      stale: true,
    });
  });
});

describe("the stale marker", () => {
  /// Render the mobile hook against one canned `connection_state`
  /// report, and return the `ConnectionState` it produces.
  async function report(payload: Record<string, unknown>) {
    const useConnectionState = await load("mobile");
    mockIPC((cmd, args) => {
      // The companion's own commands go through `remote.ts`, which
      // invokes them directly rather than wrapping them in
      // `remote_call`.
      if (cmd === "connection_state") return payload;
      if (cmd === "subscribe_events") return null;
      throw new Error(`unexpected ${cmd} ${JSON.stringify(args)}`);
    });
    const { result } = renderHook(() => useConnectionState(), { wrapper });
    await waitFor(() => expect(result.current.kind).not.toBe("unknown"));
    return result.current;
  }

  it("carries the companion's stale flag onto the state", async () => {
    // The bug: `fromReport` dropped this field, so nothing downstream
    // could tell a cached list from a live one.
    const state = await report({
      state: "unreachable",
      desktop: "octocat's laptop",
      last_poll: null,
      stale: true,
    });
    expect(state.kind).toBe("unreachable");
    expect(isStale(state)).toBe(true);
  });

  it("keeps a healthy connected desktop fresh", async () => {
    const state = await report({
      state: "connected",
      desktop: "octocat's laptop",
      last_poll: null,
      protocol_version: 2,
      stale: false,
    });
    expect(isStale(state)).toBe(false);
  });

  it("marks a connected desktop stale when the companion says so", async () => {
    // Reachable but undriveable -- below the required protocol, say.
    // The desktop answers, and its answers still must not be acted on.
    const state = await report({
      state: "connected",
      desktop: "octocat's laptop",
      last_poll: null,
      protocol_version: 1,
      stale: true,
    });
    expect(isStale(state)).toBe(true);
  });

  it("treats an unreachable desktop as stale when the field is absent", async () => {
    // A report from before the field existed still describes a desktop
    // the phone cannot reach. Defaulting THAT to fresh would mark
    // hours-old rows live, which is the failure this whole change is
    // about -- so absent means stale everywhere but `connected`.
    const state = await report({
      state: "unreachable",
      desktop: "octocat's laptop",
      last_poll: null,
    });
    expect(isStale(state)).toBe(true);
  });
});
