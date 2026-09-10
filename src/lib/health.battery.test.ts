/// The battery panel's wording and its power series (#772, #773).
///
/// Its own file rather than more cases in `SystemHealthPage.test.tsx`:
/// that file renders a component, and everything here is a pure
/// function of a number. The failure #772 is about -- a healthy battery
/// described as degraded -- is a decision about which sentence is TRUE
/// at a given reading, which needs no DOM to demonstrate.

import { describe, expect, it } from "vitest";
import {
  CAPACITY_BAR_MAX,
  barColor,
  capacityBarFill,
  capacityColor,
  capacityMeaning,
  capacityStanding,
  cycleMeaning,
  formatWatts,
  peakWatts,
  powerColor,
  powerDirection,
  powerSeries,
} from "./health";

describe("capacity above design is not wear (#772)", () => {
  /// THE #772 bug, as an assertion.
  ///
  /// The reported reading was 103% on a new laptop, under copy saying
  /// the battery "still charges to 100%, it just holds less than it
  /// once did". Both halves of that are false above 100, and the
  /// second is the one that tells a user their new battery has
  /// degraded.
  it("never tells a battery above 100% that it holds less than it once did", () => {
    for (const p of [100.6, 103, 105.4]) {
      const said = capacityMeaning(p);
      expect(said).not.toMatch(/holds less/i);
      expect(said).not.toMatch(/still charges to 100/i);
      // And it says the true thing instead.
      expect(said).toMatch(/more than/i);
      expect(said).toMatch(/no wear/i);
    }
  });

  /// A cell at exactly its nameplate is its own case.
  ///
  /// Not "above" -- there is nothing extra to report -- and emphatically
  /// not "worn", which is what a two-way split would make it.
  it("reports no wear at exactly 100%, without claiming a surplus", () => {
    expect(capacityStanding(100)).toBe("at");
    const said = capacityMeaning(100);
    expect(said).toMatch(/no wear/i);
    expect(said).not.toMatch(/more than/i);
    expect(said).not.toMatch(/holds less/i);
  });

  /// The original sentence was always correct below 100 and stays.
  it("keeps the existing wording for a genuinely worn cell", () => {
    const said = capacityMeaning(84);
    expect(said).toMatch(/holds less than it once did/i);
    expect(capacityStanding(84)).toBe("worn");
  });

  /// The wording rounds the way the panel prints.
  ///
  /// The panel shows `toFixed(0)`, so 100.4% displays as "100%". A
  /// sentence saying the cell holds MORE than its rating, printed
  /// beside a number reading exactly 100, argues with itself.
  it("does not claim a surplus for a reading that displays as 100%", () => {
    expect(100.4.toFixed(0)).toBe("100");
    expect(capacityStanding(100.4)).toBe("at");
    expect(capacityMeaning(100.4)).not.toMatch(/more than/i);
  });

  /// The cycle sentence presumed wear too, one clause further on.
  it("does not call cycles on a healthy cell ordinary ageing", () => {
    expect(cycleMeaning(103, 12)).not.toMatch(/wear|ageing/i);
    expect(cycleMeaning(103, 12)).toMatch(/12 charge cycles/i);
    // Below 100 the original sentence is still the right one.
    expect(cycleMeaning(84, 414)).toMatch(/wear after 414 cycles is ordinary/i);
  });

  it("says nothing at all when the cycle count is absent", () => {
    expect(cycleMeaning(103, null)).toBeNull();
    expect(cycleMeaning(84, null)).toBeNull();
  });

  it("counts one cycle in the singular", () => {
    expect(cycleMeaning(103, 1)).toMatch(/1 charge cycle\b/);
  });
});

describe("the capacity visual survives a reading over 100 (#772)", () => {
  /// The other half of #772: the bar must not overflow or read as an
  /// error above 100.
  ///
  /// `barColor` -- the helper every other bar on this page uses --
  /// treats a high percentage as PRESSURE and turns it red. Pinned
  /// here as the thing capacity must not do, so a future edit that
  /// "simplifies" the two into one function fails rather than turning
  /// every new battery crimson.
  it("colours a healthy cell green where the pressure scale would call it critical", () => {
    expect(barColor(103)).toBe("#f85149");
    expect(capacityColor(103)).toBe("#3fb950");
    expect(capacityColor(100)).toBe("#3fb950");
    expect(capacityColor(84)).toBe("#3fb950");
  });

  /// The bands escalate DOWNWARD, which is the inversion that makes
  /// this a separate function. Ordered wrongly, every band after the
  /// first is unreachable.
  it("escalates as capacity falls, not as it rises", () => {
    expect(capacityColor(79)).toBe("#d29922");
    expect(capacityColor(49)).toBe("#f85149");
  });

  /// The fill never exceeds its track.
  it("keeps a reading over 100% inside the bar", () => {
    const fill = capacityBarFill(103);
    expect(fill).toBeLessThan(100);
    expect(fill).toBeGreaterThan(0);
    // Even an absurd reading is clamped rather than overflowing the
    // container, which is what "must not overflow or wrap" requires.
    expect(capacityBarFill(400)).toBe(100);
    expect(capacityBarFill(-5)).toBe(0);
  });

  /// 100% sits visibly short of the end of the track, which is what
  /// leaves room for a healthy cell to sit past it.
  it("leaves headroom above the rated figure", () => {
    expect(CAPACITY_BAR_MAX).toBeGreaterThan(100);
    expect(capacityBarFill(100)).toBeLessThan(100);
    expect(capacityBarFill(103)).toBeGreaterThan(capacityBarFill(100));
  });
});

describe("the power flow (#773)", () => {
  /// The sign carries the direction, in the text as well as the data.
  it("prints a discharge as a negative wattage", () => {
    expect(formatWatts(12.76)).toBe("12.8 W");
    expect(formatWatts(-12.76)).toBe("-12.8 W");
  });

  /// `-0.0 W` reads as a typo. Zero has no direction.
  it("never prints a negative zero", () => {
    expect(formatWatts(-0.01)).toBe("0.0 W");
    expect(formatWatts(0)).toBe("0.0 W");
  });

  /// A topped-up battery on mains hovers around zero and would flicker
  /// between "charging" and "discharging" every five-second poll.
  it("calls a flow inside the dead band idle rather than flapping", () => {
    expect(powerDirection(0.02)).toBe("idle");
    expect(powerDirection(-0.02)).toBe("idle");
    expect(powerDirection(12.8)).toBe("charging");
    expect(powerDirection(-12.8)).toBe("discharging");
  });

  /// The one reading on this panel that is a FAULT, not a state: #720's
  /// condition. Discharging on battery is completely ordinary and must
  /// not be red.
  it("reserves red for discharging while plugged in", () => {
    expect(powerColor(-12.8, true)).toBe("#f85149");
    expect(powerColor(-12.8, false)).toBe("#d29922");
    expect(powerColor(12.8, true)).toBe("#3fb950");
    expect(powerColor(0, true)).toBe("#8b949e");
  });

  /// The axis has to hold both directions against one ceiling, or a
  /// charge and a discharge in the same series are drawn to different
  /// scales and cannot be compared.
  it("takes the peak from the magnitude, not the value", () => {
    const points = [
      { t: 1, v: 12.0 },
      { t: 2, v: -60.0 },
      { t: 3, v: null },
    ];
    expect(peakWatts(points)).toBe(60);
    expect(peakWatts([{ t: 1, v: null }])).toBeNull();
    expect(peakWatts([])).toBeNull();
  });

  /// Absent is not zero, in the series as everywhere else.
  ///
  /// A sample from before #773 shipped carries no `power` key at all,
  /// and one from a platform that does not publish it carries `null`.
  /// Both are unknowns the chart must break its line across -- NOT zero
  /// watts, which is a real state a full plugged-in battery sits in.
  it("reads a missing power flow as absent rather than as zero watts", () => {
    const series = powerSeries([
      { sampled_at: "2026-01-01T00:00:00Z", battery: { power: { watts: -8.5 } } },
      // The version-skew case: an older sample with no such key.
      { sampled_at: "2026-01-01T00:01:00Z", battery: {} },
      // A platform that publishes no flow.
      { sampled_at: "2026-01-01T00:02:00Z", battery: { power: null } },
      // No battery at all.
      { sampled_at: "2026-01-01T00:03:00Z", battery: null },
      // And a genuine zero, which must survive as a reading.
      { sampled_at: "2026-01-01T00:04:00Z", battery: { power: { watts: 0 } } },
    ]);
    expect(series.map((p) => p.v)).toEqual([-8.5, null, null, null, 0]);
  });

  /// An unparseable timestamp yields NaN rather than silently sliding
  /// the point to the epoch, which `splitOnGaps` then breaks on.
  it("carries each sample's own timestamp", () => {
    const series = powerSeries([
      { sampled_at: "2026-01-01T00:00:00Z", battery: { power: { watts: 5 } } },
    ]);
    expect(series[0].t).toBe(Date.parse("2026-01-01T00:00:00Z"));
  });
});
