import { describe, expect, it, vi, beforeEach, afterEach } from "vitest";
import { act, cleanup, renderHook, waitFor } from "@testing-library/react";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import type { ReactNode } from "react";
import type { PairingRequest } from "./tauri";

const invoke = vi.hoisted(() => vi.fn<(...a: unknown[]) => Promise<unknown>>());
/// The most recent `pairing-request` handler, so a test can fire the
/// event the Rust side would.
const handlers = vi.hoisted(() => new Map<string, (e: { payload: unknown }) => void>());
vi.mock("@tauri-apps/api/core", () => ({ invoke }));
vi.mock("@tauri-apps/api/event", () => ({
  listen: vi.fn((name: string, cb: (e: { payload: unknown }) => void) => {
    handlers.set(name, cb);
    return Promise.resolve(() => handlers.delete(name));
  }),
}));

import {
  useIssuePairingToken,
  usePairedDevices,
  usePairingRequest,
  useRespondToPairing,
  useRevokePairedDevice,
} from "./hooks";

const PAYLOAD = {
  v: 1,
  name: "octocat's laptop",
  addrs: ["192.0.2.10"],
  port: 41919,
  fp: "sha256:" + "ab".repeat(32),
  token: "dG9rZW4",
  exp: 1_757_068_800,
};

const DEVICE = {
  id: 1,
  name: "Octocat's phone",
  cert_fp: "cd".repeat(32),
  has_mldsa: true,
  paired_at: "2026-09-01T10:00:00Z",
  last_seen: null,
};

const REQUEST: PairingRequest = {
  request_id: 7,
  device_name: "Octocat's phone",
  fingerprint: "cd".repeat(32),
  has_mldsa: false,
};

const calls = (cmd: string) => invoke.mock.calls.filter(([c]) => c === cmd);

function harness() {
  const qc = new QueryClient({ defaultOptions: { queries: { retry: false } } });
  const wrapper = ({ children }: { children: ReactNode }) => (
    <QueryClientProvider client={qc}>{children}</QueryClientProvider>
  );
  return { qc, wrapper };
}

beforeEach(() => {
  invoke.mockReset();
  handlers.clear();
  invoke.mockImplementation((cmd) => {
    switch (cmd) {
      case "get_remote_enabled":
        return Promise.resolve(false);
      case "issue_pairing_token":
        return Promise.resolve(PAYLOAD);
      case "list_paired_devices":
        return Promise.resolve([DEVICE]);
      default:
        return Promise.resolve(undefined);
    }
  });
});
afterEach(cleanup);

describe("usePairedDevices", () => {
  it("lists what the desktop has paired", async () => {
    const { wrapper } = harness();
    const { result } = renderHook(() => usePairedDevices(), { wrapper });
    await waitFor(() => expect(result.current.data).toEqual([DEVICE]));
  });
});

/// **Pair a phone** must work from a cold start: the listener is off by
/// default, and a QR code for a port nothing is listening on would send
/// the phone to a connection refused.
describe("useIssuePairingToken", () => {
  it("turns the listener on first when it is off, then mints the token", async () => {
    const { wrapper } = harness();
    const { result } = renderHook(() => useIssuePairingToken(), { wrapper });
    await expect(result.current()).resolves.toEqual(PAYLOAD);
    expect(invoke).toHaveBeenCalledWith("set_remote_enabled", { enabled: true });
    // Order matters: the token is minted only once the port is bound.
    const order = invoke.mock.calls.map(([c]) => c);
    expect(order.indexOf("set_remote_enabled")).toBeLessThan(
      order.indexOf("issue_pairing_token"),
    );
  });

  it("leaves a running listener alone", async () => {
    invoke.mockImplementation((cmd) =>
      Promise.resolve(cmd === "get_remote_enabled" ? true : PAYLOAD),
    );
    const { wrapper } = harness();
    const { result } = renderHook(() => useIssuePairingToken(), { wrapper });
    await result.current();
    expect(calls("set_remote_enabled")).toHaveLength(0);
    expect(calls("issue_pairing_token")).toHaveLength(1);
  });

  it("refreshes the Allow phone connections box after turning the listener on", async () => {
    const { qc, wrapper } = harness();
    qc.setQueryData(["remote-enabled"], false);
    const { result } = renderHook(() => useIssuePairingToken(), { wrapper });
    await result.current();
    expect(qc.getQueryState(["remote-enabled"])?.isInvalidated).toBe(true);
  });

  it("does not mint a token when the listener refuses to start", async () => {
    invoke.mockImplementation((cmd) => {
      if (cmd === "set_remote_enabled") return Promise.reject("could not listen on 0.0.0.0:41919: in use");
      return Promise.resolve(cmd === "get_remote_enabled" ? false : PAYLOAD);
    });
    const { wrapper } = harness();
    const { result } = renderHook(() => useIssuePairingToken(), { wrapper });
    await expect(result.current()).rejects.toBe("could not listen on 0.0.0.0:41919: in use");
    expect(calls("issue_pairing_token")).toHaveLength(0);
  });
});

describe("useRespondToPairing", () => {
  it("passes the decision through and refreshes the list on approve", async () => {
    const { qc, wrapper } = harness();
    qc.setQueryData(["paired-devices"], []);
    const { result } = renderHook(() => useRespondToPairing(), { wrapper });
    await result.current(7, true);
    expect(invoke).toHaveBeenCalledWith("respond_to_pairing", {
      requestId: 7,
      approve: true,
      replaceExisting: null,
    });
    expect(qc.getQueryState(["paired-devices"])?.isInvalidated).toBe(true);
  });

  it("forwards the same-name answer", async () => {
    const { wrapper } = harness();
    const { result } = renderHook(() => useRespondToPairing(), { wrapper });
    await result.current(7, true, false);
    expect(invoke).toHaveBeenCalledWith("respond_to_pairing", {
      requestId: 7,
      approve: true,
      replaceExisting: false,
    });
  });

  it("surfaces the name-taken refusal so the modal can ask", async () => {
    invoke.mockImplementation((cmd) =>
      cmd === "respond_to_pairing"
        ? Promise.reject('a device named "Octocat\'s phone" is already paired')
        : Promise.resolve(undefined),
    );
    const { wrapper } = harness();
    const { result } = renderHook(() => useRespondToPairing(), { wrapper });
    await expect(result.current(7, true)).rejects.toMatch(/already paired/);
  });
});

describe("useRevokePairedDevice", () => {
  it("revokes, then refreshes the list", async () => {
    const { qc, wrapper } = harness();
    qc.setQueryData(["paired-devices"], [DEVICE]);
    const { result } = renderHook(() => useRevokePairedDevice(), { wrapper });
    await result.current(1);
    expect(invoke).toHaveBeenCalledWith("revoke_paired_device", { id: 1 });
    expect(qc.getQueryState(["paired-devices"])?.isInvalidated).toBe(true);
  });
});

/// The event can arrive with Settings closed, or twice in a row. The
/// hook holds a queue and hands out one request at a time; a second
/// phone waits for the first decision rather than replacing it on
/// screen mid-comparison.
describe("usePairingRequest", () => {
  it("starts with nothing pending", async () => {
    const { result } = renderHook(() => usePairingRequest());
    expect(result.current.request).toBeNull();
    await waitFor(() => expect(handlers.has("pairing-request")).toBe(true));
  });

  it("surfaces the event payload, one at a time, in arrival order", async () => {
    const { result } = renderHook(() => usePairingRequest());
    await waitFor(() => expect(handlers.has("pairing-request")).toBe(true));
    const fire = handlers.get("pairing-request")!;
    act(() => fire({ payload: REQUEST }));
    act(() => fire({ payload: { ...REQUEST, request_id: 8, device_name: "Second" } }));
    expect(result.current.request).toEqual(REQUEST);
    act(() => result.current.dismiss());
    expect(result.current.request?.request_id).toBe(8);
    act(() => result.current.dismiss());
    expect(result.current.request).toBeNull();
  });

  it("ignores a second delivery of the same request", async () => {
    const { result } = renderHook(() => usePairingRequest());
    await waitFor(() => expect(handlers.has("pairing-request")).toBe(true));
    const fire = handlers.get("pairing-request")!;
    act(() => fire({ payload: REQUEST }));
    act(() => fire({ payload: REQUEST }));
    act(() => result.current.dismiss());
    expect(result.current.request).toBeNull();
  });

  it("stops listening on unmount", async () => {
    const { unmount } = renderHook(() => usePairingRequest());
    await waitFor(() => expect(handlers.has("pairing-request")).toBe(true));
    unmount();
    await waitFor(() => expect(handlers.has("pairing-request")).toBe(false));
  });
});
