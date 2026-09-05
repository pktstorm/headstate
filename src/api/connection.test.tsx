import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { renderHook, waitFor } from "@testing-library/react";
import { clearMocks, mockIPC } from "@tauri-apps/api/mocks";
import type { ReactNode } from "react";
import { afterEach, describe, expect, it, vi } from "vitest";

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
        ? { state: "connected", desktop: "octocat's laptop", last_poll: "2026-09-04T10:00:00Z" }
        : undefined,
    );
    const { result } = renderHook(() => useConnectionState(), { wrapper });
    await waitFor(() =>
      expect(result.current).toEqual({
        kind: "connected",
        desktop: "octocat's laptop",
        lastPoll: "2026-09-04T10:00:00Z",
      }),
    );
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
    expect(result.current).toEqual({ kind: "unreachable", desktop: "Desktop", lastPoll: null });
  });
});
