import { cleanup, render, screen } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import { stubViewport } from "@/test-utils";

vi.mock("./ViewSwitcher", () => ({ ViewSwitcher: () => null }));

import { DockerSidebar } from "./DockerSidebar";

afterEach(() => {
  cleanup();
  stubViewport(null);
});

describe("DockerSidebar", () => {
  it("offers Images and Stats on the desktop", () => {
    stubViewport(1400);
    render(<DockerSidebar />);
    expect(screen.getByRole("button", { name: /images/i })).toBeTruthy();
    expect(screen.getByRole("button", { name: /stats/i })).toBeTruthy();
  });

  it("drops Stats on a phone, which has no stats page", () => {
    stubViewport(390);
    render(<DockerSidebar />);
    expect(screen.getByRole("button", { name: /images/i })).toBeTruthy();
    expect(screen.queryByRole("button", { name: /stats/i })).toBeNull();
  });
});
