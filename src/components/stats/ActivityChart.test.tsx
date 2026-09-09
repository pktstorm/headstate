import { fireEvent, render, screen } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";
import { ActivityChart } from "./ActivityChart";

const points = [
  { date: "2026-08-18", opened: 10, merged: 8 },
  { date: "2026-08-19", opened: 12, merged: 14 },
];

describe("ActivityChart", () => {
  it("offers the three range toggles", () => {
    render(<ActivityChart points={points} days={30} onDaysChange={() => {}} />);
    expect(screen.getByRole("button", { name: "7d" })).toBeTruthy();
    expect(screen.getByRole("button", { name: "14d" })).toBeTruthy();
    expect(screen.getByRole("button", { name: "30d" })).toBeTruthy();
  });

  it("marks the active range", () => {
    render(<ActivityChart points={points} days={14} onDaysChange={() => {}} />);
    expect(
      screen.getByRole("button", { name: "14d" }).getAttribute("aria-pressed"),
    ).toBe("true");
    expect(
      screen.getByRole("button", { name: "30d" }).getAttribute("aria-pressed"),
    ).toBe("false");
  });

  it("reports a range change", () => {
    const onDaysChange = vi.fn();
    render(<ActivityChart points={points} days={30} onDaysChange={onDaysChange} />);
    fireEvent.click(screen.getByRole("button", { name: "7d" }));
    expect(onDaysChange).toHaveBeenCalledWith(7);
  });

  // The buckets are UTC and the label must say so: a Pacific user's
  // evening merge lands in the next day's column, and an undisclosed
  // off-by-one-day chart is worse than a labelled one.
  it("discloses that days are UTC", () => {
    render(<ActivityChart points={points} days={30} onDaysChange={() => {}} />);
    expect(screen.getByText(/\(UTC\)/)).toBeTruthy();
  });

  it("shows an empty state rather than a broken axis", () => {
    render(<ActivityChart points={[]} days={30} onDaysChange={() => {}} />);
    expect(screen.getByText(/no activity/i)).toBeTruthy();
  });

  // #3fb950 and #58a6ff measure 1.01:1 against each other, so with hue
  // removed the two areas are the same shade. The tooltip does name them,
  // but it is a hover affordance -- no use to a keyboard, to touch, or to
  // anyone reading the chart rather than pointing at it.
  it("names both series in a static key, not only in the tooltip", () => {
    render(<ActivityChart points={points} days={30} onDaysChange={() => {}} />);
    expect(screen.getByText("Opened")).toBeTruthy();
    expect(screen.getByText("Merged")).toBeTruthy();
  });

  // The key is only honest if the swatches match the lines, and the dash
  // is what tells the series apart once colour is gone.
  it("distinguishes the series by stroke dash as well as colour", () => {
    const { container } = render(
      <ActivityChart points={points} days={30} onDaysChange={() => {}} />,
    );
    const dashed = container.querySelectorAll('[stroke-dasharray="4 3"]');
    // One in the key's swatch, one on the series itself.
    expect(dashed.length).toBeGreaterThanOrEqual(2);
  });

  // The chart is the centrepiece; a silent render failure would leave an
  // empty card that still looks deliberate.
  it("actually draws the series", () => {
    const { container } = render(
      <ActivityChart points={points} days={30} onDaysChange={() => {}} />,
    );
    expect(container.querySelector("svg")).toBeTruthy();
    expect(container.querySelectorAll("path").length).toBeGreaterThan(0);
  });
});
