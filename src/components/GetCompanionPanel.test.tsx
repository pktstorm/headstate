import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, describe, expect, it } from "vitest";

import { GetCompanionPanel } from "./GetCompanionPanel";

/// A realistic public TestFlight link, in the shape Apple issues.
/// Synthetic: this repo is public and CONTRIBUTING.md keeps real
/// identifiers out. The code is not a real invitation.
const JOIN = "https://testflight.apple.com/join/abc12345";

afterEach(cleanup);

/// Every test past the first needs the instructions visible.
function open() {
  fireEvent.click(screen.getByRole("button", { name: /get the mobile companion app/i }));
}

describe("GetCompanionPanel", () => {
  it("keeps the instructions collapsed until asked", () => {
    render(<GetCompanionPanel joinUrl={JOIN} />);

    // The button is the whole affordance; the steps are noise until
    // someone wants them.
    expect(
      screen
        .getByRole("button", { name: /get the mobile companion app/i })
        .getAttribute("aria-expanded"),
    ).toBe("false");
    expect(screen.queryByText(/TestFlight app from the App Store/i)).toBeNull();
  });

  it("gives the three TestFlight steps in order once opened", () => {
    render(<GetCompanionPanel joinUrl={JOIN} />);
    open();

    // Order is load-bearing: TestFlight has to be installed BEFORE the
    // invitation is opened, or the invitation is spent on an App Store
    // page for TestFlight itself. Asserting on the rendered sequence,
    // not merely on presence.
    const steps = screen.getAllByRole("listitem").map((li) => li.textContent ?? "");
    expect(steps).toHaveLength(3);
    expect(steps[0]).toMatch(/install Apple.s TestFlight app/i);
    expect(steps[1]).toMatch(/invitation/i);
    expect(steps[2]).toMatch(/Accept/);
    expect(steps[2]).toMatch(/Install/);
  });

  it("shows a QR code carrying the join link, and the link itself", () => {
    render(<GetCompanionPanel joinUrl={JOIN} />);
    open();

    // The QR is the point of the panel: the invitation must open on the
    // phone while the user is reading this on a Mac.
    const qr = screen.getByRole("img", {
      name: /QR code linking to the TestFlight beta invitation/i,
    });
    // A QR actually rendered. The encoded value is not readable from the
    // SVG without decoding it, so the link shown beside it is what pins
    // the URL -- and both come from the same `joinUrl`, so a wrong URL
    // cannot reach the code while leaving the text right.
    expect(qr.querySelector("svg")).toBeTruthy();

    // And the link stays readable, for anyone who cannot scan.
    expect(screen.getByText(JOIN)).toBeTruthy();
  });

  it("says the invitation is on the phone, not this Mac", () => {
    render(<GetCompanionPanel joinUrl={JOIN} />);
    open();

    // The mistake the wording exists to prevent: opening the link on
    // the desktop, where TestFlight cannot act on it.
    expect(screen.getByText(/not on this Mac/i)).toBeTruthy();
  });

  it("says there is nothing to scan yet when no invitation is configured", () => {
    // The state that ships first: #736 is a console task on Apple's
    // side, so an empty link is real and will be seen.
    render(<GetCompanionPanel joinUrl="" />);
    open();

    expect(screen.getByText(/not available yet/i)).toBeTruthy();
    // A QR code pointing nowhere would send someone to a dead page
    // having already installed TestFlight, with no way to tell whether
    // they had done something wrong.
    expect(
      screen.queryByRole("img", { name: /QR code linking to the TestFlight/i }),
    ).toBeNull();
  });
});
