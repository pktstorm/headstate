import { act, cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

const FP = "ab12cd34ef5601237890abcdef0123456789abcdef0123456789abcdef012345";
const NOW = new Date("2026-09-05T10:00:00Z").getTime();

const payload = (exp: number) => ({
  v: 1 as const,
  name: "octocat's laptop",
  addrs: ["192.0.2.10"],
  port: 41919,
  fp: `sha256:${FP}`,
  token: "dG9rZW4",
  exp,
});

const issue = vi.hoisted(() => vi.fn<() => Promise<unknown>>());
vi.mock("../api/hooks", () => ({ useIssuePairingToken: () => issue }));

import { PairPhonePanel } from "./PairPhonePanel";

beforeEach(() => {
  vi.useFakeTimers();
  vi.setSystemTime(NOW);
  issue.mockReset();
  issue.mockImplementation(() => Promise.resolve(payload(NOW / 1000 + 120)));
});
afterEach(() => {
  cleanup();
  vi.useRealTimers();
});

const start = async () => {
  render(<PairPhonePanel />);
  await act(async () => {
    fireEvent.click(screen.getByRole("button", { name: /pair a phone/i }));
  });
};

const qr = () => screen.queryByRole("img", { name: /pairing qr code/i });

describe("PairPhonePanel", () => {
  it("shows only the button until asked", () => {
    render(<PairPhonePanel />);
    expect(screen.getByRole("button", { name: /pair a phone/i })).toBeTruthy();
    expect(qr()).toBeNull();
    expect(issue).not.toHaveBeenCalled();
  });

  it("renders the QR code and the fingerprint in groups of four", async () => {
    await start();
    expect(issue).toHaveBeenCalledTimes(1);
    expect(qr()).toBeTruthy();
    expect(qr()!.querySelector("svg")).toBeTruthy();
    expect(screen.getByText(/SHA256/)).toBeTruthy();
    expect(
      screen.getByText(
        "ab12 cd34 ef56 0123 7890 abcd ef01 2345 6789 abcd ef01 2345 6789 abcd ef01 2345",
      ),
    ).toBeTruthy();
  });

  // The countdown is the token's `exp`, not "two minutes from now": a
  // slow command would otherwise promise seconds the token does not have.
  it("counts down from the payload's expiry", async () => {
    issue.mockImplementation(() => Promise.resolve(payload(NOW / 1000 + 90)));
    await start();
    expect(screen.getByText(/expires in/i).textContent).toMatch(/1:30$/);
    act(() => vi.advanceTimersByTime(1000));
    expect(screen.getByText(/expires in/i).textContent).toMatch(/1:29$/);
  });

  it("blanks the code on expiry and offers Regenerate", async () => {
    await start();
    expect(screen.getByText(/expires in/i).textContent).toMatch(/2:00$/);
    act(() => vi.advanceTimersByTime(120_000));
    expect(qr()).toBeNull();
    expect(screen.getByText(/this code has expired/i)).toBeTruthy();
    expect(screen.queryByText(/expires in/i)).toBeNull();
    expect(screen.getByRole("button", { name: /regenerate/i })).toBeTruthy();
  });

  it("Regenerate mints a new token and restarts the countdown", async () => {
    await start();
    act(() => vi.advanceTimersByTime(120_000));
    issue.mockImplementation(() =>
      Promise.resolve({ ...payload(Date.now() / 1000 + 120), token: "c2Vjb25k" }),
    );
    await act(async () => {
      fireEvent.click(screen.getByRole("button", { name: /regenerate/i }));
    });
    expect(issue).toHaveBeenCalledTimes(2);
    expect(qr()).toBeTruthy();
    expect(screen.getByText(/expires in/i).textContent).toMatch(/2:00$/);
  });

  it("Regenerate works before expiry too", async () => {
    await start();
    act(() => vi.advanceTimersByTime(30_000));
    issue.mockImplementation(() =>
      Promise.resolve({ ...payload(Date.now() / 1000 + 120), token: "c2Vjb25k" }),
    );
    await act(async () => {
      fireEvent.click(screen.getByRole("button", { name: /regenerate/i }));
    });
    expect(screen.getByText(/expires in/i).textContent).toMatch(/2:00$/);
  });

  it("shows the backend's refusal", async () => {
    issue.mockImplementation(() =>
      Promise.reject("this desktop has no certificate yet; phone pairing is not available"),
    );
    await start();
    expect(screen.getByRole("alert").textContent).toBe(
      "this desktop has no certificate yet; phone pairing is not available",
    );
    expect(qr()).toBeNull();
  });

  it("Hide puts the button back", async () => {
    await start();
    fireEvent.click(screen.getByRole("button", { name: /hide/i }));
    expect(qr()).toBeNull();
    expect(screen.getByRole("button", { name: /pair a phone/i })).toBeTruthy();
  });
});
