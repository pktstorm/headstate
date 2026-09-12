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
  /// Images is a HEADING now, not a button (#852).
  ///
  /// It was a `<button>` whose entire effect was `setPanel("list")` --
  /// already the only value anything read -- carrying a bare
  /// `aria-pressed`, which with no value means `"true"`: a
  /// permanently-pressed toggle that could never be un-pressed. And
  /// `aria-pressed` is the wrong attribute regardless, for the reason
  /// `SystemHealthSidebar` states: "these are navigation, not toggles, and
  /// a screen reader announcing 'pressed' for the page you are already on
  /// describes a control that did something rather than a location you are
  /// at."
  ///
  /// With one destination there is nothing to navigate between, so the
  /// honest element is no button at all. The ZERO-button assertion is the
  /// one that matters: the defect was a control that consumed a click and
  /// reported that nothing happened.
  it("names what the column shows without offering a control, on the desktop", () => {
    stubViewport(1400);
    render(<DockerSidebar />);
    expect(screen.getByText(/images/i)).toBeTruthy();
    expect(screen.queryAllByRole("button")).toHaveLength(0);
    // And no toggle semantics anywhere: the old `aria-pressed` announced a
    // pressed state that could not be changed.
    expect(document.querySelector("[aria-pressed]")).toBeNull();
  });

  it("renders exactly the same at a phone width", () => {
    stubViewport(390);
    render(<DockerSidebar />);
    expect(screen.getByText(/images/i)).toBeTruthy();
    expect(screen.queryAllByRole("button")).toHaveLength(0);
  });
});
