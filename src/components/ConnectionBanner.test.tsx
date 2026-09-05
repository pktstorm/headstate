import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import type { ConnectionState } from "@/api/connection";
import { stubViewport } from "@/test-utils";

const connection = vi.hoisted(() => ({ current: { kind: "local" } as ConnectionState }));

vi.mock("@/api/connection", () => ({
  useConnectionState: () => connection.current,
}));

// The banner opens Settings, which reads every preference hook. None of
// them matter here beyond not throwing.
vi.mock("./SettingsDialog", () => ({
  SettingsDialog: ({ initialSection }: { initialSection?: string }) => (
    <div role="dialog">Settings: {initialSection}</div>
  ),
}));

import { ConnectionBanner } from "./ConnectionBanner";

afterEach(() => {
  cleanup();
  stubViewport(null);
  connection.current = { kind: "local" };
});

describe("ConnectionBanner", () => {
  it("renders nothing on the desktop layout", () => {
    stubViewport(1400);
    connection.current = { kind: "connected", desktop: "octocat's laptop", lastPoll: null };
    const { container } = render(<ConnectionBanner />);
    expect(container.innerHTML).toBe("");
  });

  it("renders nothing while the connection is local, even when narrow", () => {
    // A desktop build in a narrow dev browser has no desktop to name.
    stubViewport(390);
    connection.current = { kind: "local" };
    const { container } = render(<ConnectionBanner />);
    expect(container.innerHTML).toBe("");
  });

  it("names the desktop, says it is reachable, and shows the last poll", () => {
    stubViewport(390);
    const tenMinutesAgo = new Date(Date.now() - 10 * 60_000).toISOString();
    connection.current = {
      kind: "connected",
      desktop: "octocat's laptop",
      lastPoll: tenMinutesAgo,
    };
    render(<ConnectionBanner />);
    const banner = screen.getByRole("button", { name: /octocat's laptop/ });
    expect(banner.textContent).toContain("reachable");
    expect(banner.textContent).toContain("last poll 10 minutes ago");
  });

  it("says when the desktop is away and when it was last seen", () => {
    stubViewport(390);
    connection.current = {
      kind: "unreachable",
      desktop: "octocat's laptop",
      lastPoll: new Date(Date.now() - 2 * 3_600_000).toISOString(),
    };
    render(<ConnectionBanner />);
    expect(screen.getByRole("button").textContent).toContain(
      "octocat's laptop is unreachable · last seen 2 hours ago",
    );
  });

  it("invites pairing when there is no desktop", () => {
    stubViewport(390);
    connection.current = { kind: "unpaired" };
    render(<ConnectionBanner />);
    expect(screen.getByRole("button").textContent).toMatch(/not paired/i);
  });

  it("admits when it cannot ask, rather than claiming a state", () => {
    stubViewport(390);
    connection.current = { kind: "unknown" };
    render(<ConnectionBanner />);
    expect(screen.getByRole("button").textContent).toMatch(/unavailable/i);
  });

  it("opens Settings on the Phone topic when tapped", async () => {
    stubViewport(390);
    connection.current = { kind: "unpaired" };
    render(<ConnectionBanner />);
    expect(screen.queryByRole("dialog")).toBeNull();
    fireEvent.click(screen.getByRole("button"));
    await waitFor(() => expect(screen.getByRole("dialog").textContent).toContain("phone"));
  });
});
