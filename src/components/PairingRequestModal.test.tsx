import { act, cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { PairingRequest } from "../api/tauri";

const FP = "ab12cd34ef5601237890abcdef0123456789abcdef0123456789abcdef012345";

const respond = vi.hoisted(() =>
  vi.fn<(id: number, approve: boolean, replace?: boolean) => Promise<void>>(),
);
const pending = vi.hoisted(() => ({
  request: null as PairingRequest | null,
  dismiss: vi.fn(),
}));
vi.mock("../api/hooks", () => ({
  useRespondToPairing: () => respond,
  usePairingRequest: () => pending,
}));

import { PairingRequestDialog, PairingRequestModal } from "./PairingRequestModal";

const REQUEST: PairingRequest = {
  request_id: 7,
  device_name: "Octocat's phone",
  fingerprint: FP,
  has_mldsa: true,
};

beforeEach(() => {
  vi.useFakeTimers();
  vi.setSystemTime(new Date("2026-09-05T10:00:00Z"));
  respond.mockReset();
  respond.mockImplementation(() => Promise.resolve());
  pending.request = null;
  pending.dismiss.mockReset();
});
afterEach(() => {
  cleanup();
  vi.useRealTimers();
});

const onDone = vi.fn();
const show = (request: PairingRequest = REQUEST) => {
  onDone.mockReset();
  render(<PairingRequestDialog request={request} onDone={onDone} />);
};
const click = async (name: RegExp) => {
  await act(async () => {
    fireEvent.click(screen.getByRole("button", { name }));
  });
};

describe("PairingRequestDialog", () => {
  it("names the device and shows the fingerprint in groups of four", () => {
    show();
    expect(screen.getByRole("dialog").textContent).toContain("Pair Octocat's phone?");
    expect(
      screen.getByText(
        "ab12 cd34 ef56 0123 7890 abcd ef01 2345 6789 abcd ef01 2345 6789 abcd ef01 2345",
      ),
    ).toBeTruthy();
  });

  it("says a post-quantum key was offered", () => {
    show();
    expect(screen.getByText(/offered a post-quantum signing key/i)).toBeTruthy();
  });

  it("says when no post-quantum key was offered", () => {
    show({ ...REQUEST, has_mldsa: false });
    expect(screen.getByText(/no post-quantum signing key offered/i)).toBeTruthy();
  });

  it("approves and closes", async () => {
    show();
    await click(/approve/i);
    expect(respond).toHaveBeenCalledWith(7, true, undefined);
    expect(onDone).toHaveBeenCalledTimes(1);
  });

  it("denies and closes", async () => {
    show();
    await click(/deny/i);
    expect(respond).toHaveBeenCalledWith(7, false, undefined);
    expect(onDone).toHaveBeenCalledTimes(1);
  });

  it("counts down and denies on its own at two minutes", async () => {
    show();
    expect(screen.getByText(/denied automatically in/i).textContent).toMatch(/2:00$/);
    act(() => vi.advanceTimersByTime(1000));
    expect(screen.getByText(/denied automatically in/i).textContent).toMatch(/1:59$/);
    expect(respond).not.toHaveBeenCalled();
    await act(async () => {
      vi.advanceTimersByTime(119_000);
    });
    expect(respond).toHaveBeenCalledTimes(1);
    expect(respond).toHaveBeenCalledWith(7, false);
    expect(onDone).toHaveBeenCalledTimes(1);
  });

  // The Rust side has already timed out by then; a rejected deny is the
  // same outcome and must not leave a dead modal on screen.
  it("closes on timeout even when the deny is refused", async () => {
    respond.mockImplementation(() => Promise.reject("request 7 is no longer pending"));
    show();
    await act(async () => {
      vi.advanceTimersByTime(120_000);
    });
    expect(onDone).toHaveBeenCalledTimes(1);
  });

  it("keeps the request open and shows an error when the answer is refused", async () => {
    respond.mockImplementation(() => Promise.reject("database is locked"));
    show();
    await click(/approve/i);
    expect(screen.getByRole("alert").textContent).toBe("database is locked");
    expect(onDone).not.toHaveBeenCalled();
    expect(screen.getByRole("button", { name: /approve/i })).toBeTruthy();
  });

  describe("when a device with that name is already paired", () => {
    const taken = () =>
      respond.mockImplementationOnce(() =>
        Promise.reject('a device named "Octocat\'s phone" is already paired'),
      );

    it("asks before replacing", async () => {
      taken();
      show();
      await click(/approve/i);
      expect(screen.getByRole("dialog").textContent).toContain(
        "Replace the existing pairing for Octocat's phone?",
      );
      expect(screen.getByRole("button", { name: /replace/i })).toBeTruthy();
      expect(screen.getByRole("button", { name: /keep both/i })).toBeTruthy();
      expect(screen.getByRole("button", { name: /cancel/i })).toBeTruthy();
      expect(screen.queryByRole("alert")).toBeNull();
      expect(onDone).not.toHaveBeenCalled();
    });

    it("Replace answers again with replace_existing = true", async () => {
      taken();
      show();
      await click(/approve/i);
      await click(/replace/i);
      expect(respond).toHaveBeenLastCalledWith(7, true, true);
      expect(onDone).toHaveBeenCalledTimes(1);
    });

    it("Keep both answers again with replace_existing = false", async () => {
      taken();
      show();
      await click(/approve/i);
      await click(/keep both/i);
      expect(respond).toHaveBeenLastCalledWith(7, true, false);
      expect(onDone).toHaveBeenCalledTimes(1);
    });

    // The request is still pending on the Rust side, so backing out of
    // the replace question returns to the decision, not to nothing.
    it("Cancel goes back to Approve and Deny with the request still pending", async () => {
      taken();
      show();
      await click(/approve/i);
      await click(/cancel/i);
      expect(screen.getByRole("button", { name: /approve/i })).toBeTruthy();
      expect(screen.getByRole("button", { name: /deny/i })).toBeTruthy();
      expect(respond).toHaveBeenCalledTimes(1);
      expect(onDone).not.toHaveBeenCalled();
    });
  });
});

describe("PairingRequestModal", () => {
  it("renders nothing while no phone is waiting", () => {
    render(<PairingRequestModal />);
    expect(screen.queryByRole("dialog")).toBeNull();
  });

  it("shows the waiting request and dismisses it once answered", async () => {
    pending.request = REQUEST;
    render(<PairingRequestModal />);
    expect(screen.getByRole("dialog").textContent).toContain("Octocat's phone");
    await click(/deny/i);
    expect(pending.dismiss).toHaveBeenCalledTimes(1);
  });
});
