import { describe, expect, it, vi, beforeEach } from "vitest";
import { renderHook, waitFor } from "@testing-library/react";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import type { ReactNode } from "react";

const invoke = vi.hoisted(() => vi.fn<(...a: unknown[]) => Promise<unknown>>());
vi.mock("@tauri-apps/api/core", () => ({ invoke }));
vi.mock("@tauri-apps/api/event", () => ({ listen: vi.fn(() => Promise.resolve(() => {})) }));

import { useRemoteEnabled } from "./hooks";

/// **Allow phone connections** asks the listener whether it is running
/// rather than reading the stored setting, and refetches after every
/// attempt -- including a failed one, because a start that bound the
/// port and then could not save the setting is still a running
/// listener the box has to show.
describe("useRemoteEnabled", () => {
  beforeEach(() => {
    invoke.mockReset();
    invoke.mockImplementation((cmd) =>
      Promise.resolve(cmd === "get_remote_enabled" ? false : undefined),
    );
  });

  const harness = () => {
    const qc = new QueryClient({ defaultOptions: { queries: { retry: false } } });
    const wrapper = ({ children }: { children: ReactNode }) => (
      <QueryClientProvider client={qc}>{children}</QueryClientProvider>
    );
    return { qc, wrapper };
  };

  it("reads the live state and defaults to off while loading", async () => {
    const { wrapper } = harness();
    const { result } = renderHook(() => useRemoteEnabled(), { wrapper });
    expect(result.current.enabled).toBe(false);
    await waitFor(() => expect(invoke).toHaveBeenCalledWith("get_remote_enabled"));
  });

  it("sets, then refetches", async () => {
    const { wrapper } = harness();
    const { result } = renderHook(() => useRemoteEnabled(), { wrapper });
    await waitFor(() => expect(invoke).toHaveBeenCalledWith("get_remote_enabled"));
    invoke.mockImplementation((cmd) =>
      Promise.resolve(cmd === "get_remote_enabled" ? true : undefined),
    );

    await result.current.set(true);

    expect(invoke).toHaveBeenCalledWith("set_remote_enabled", { enabled: true });
    await waitFor(() => expect(result.current.enabled).toBe(true));
  });

  it("refetches even when the change was refused, and still rejects", async () => {
    const { wrapper } = harness();
    const { result } = renderHook(() => useRemoteEnabled(), { wrapper });
    await waitFor(() => expect(invoke).toHaveBeenCalledWith("get_remote_enabled"));
    const reads = invoke.mock.calls.filter(([c]) => c === "get_remote_enabled").length;
    invoke.mockImplementation((cmd) =>
      cmd === "set_remote_enabled"
        ? Promise.reject("could not listen on 0.0.0.0:41919: in use")
        : Promise.resolve(cmd === "get_remote_enabled" ? false : undefined),
    );

    await expect(result.current.set(true)).rejects.toBe(
      "could not listen on 0.0.0.0:41919: in use",
    );
    await waitFor(() =>
      expect(
        invoke.mock.calls.filter(([c]) => c === "get_remote_enabled").length,
      ).toBeGreaterThan(reads),
    );
  });
});
