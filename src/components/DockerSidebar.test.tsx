import { cleanup, render, screen } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import { stubViewport } from "@/test-utils";

vi.mock("./ViewSwitcher", () => ({ ViewSwitcher: () => null }));

import { DockerSidebar } from "./DockerSidebar";

afterEach(() => {
  cleanup();
  stubViewport(null);
});

/// The pinned Stats row is gone from this sidebar (#794). It existed only
/// because Stats was a `panel` of My PRs and therefore needed a control
/// that set both axes at once; PR Stats is a view, so `ViewSwitcher`
/// reaches it -- and this file mocks that component away, which is why
/// these assertions see nothing but Images.
///
/// Both widths are still exercised, and deliberately: the pair used to
/// encode a VIEWPORT rule (#598's mistake -- a narrow desktop window lost
/// a page it genuinely had), so asserting the two now agree is what pins
/// that the rule is gone rather than merely inverted.
describe("DockerSidebar", () => {
  it("offers Images and nothing pinned beneath it, on the desktop", () => {
    stubViewport(1400);
    render(<DockerSidebar />);
    expect(screen.getByRole("button", { name: /images/i })).toBeTruthy();
    expect(screen.getAllByRole("button")).toHaveLength(1);
  });

  it("renders exactly the same at a phone width", () => {
    stubViewport(390);
    render(<DockerSidebar />);
    expect(screen.getByRole("button", { name: /images/i })).toBeTruthy();
    expect(screen.getAllByRole("button")).toHaveLength(1);
  });
});
