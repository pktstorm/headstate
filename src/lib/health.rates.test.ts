/// Turning cumulative network counters into a rate (#719).
///
/// Its own file rather than more cases in `SystemHealthPage.test.tsx`,
/// which is where `splitOnGaps` is tested: that file renders a
/// component, and everything here is arithmetic. The two failures these
/// tests exist for -- a rate computed across a gap, and a counter reset
/// rendered as a plausible value -- are both pure functions of the
/// numbers, and neither needs a DOM to demonstrate.

import { describe, expect, it } from "vitest";
import {
  SAMPLE_INTERVAL_MS,
  counterRates,
  formatRate,
  interfaceRates,
  peakRate,
  splitOnGaps,
  type CounterSample,
} from "./health";

/// Samples one minute apart, ending now, from a list of byte counts.
///
/// `rx` and `tx` are given the same value: these tests are about
/// DIFFERENCING, and two directions would only double every assertion
/// without exercising a different path.
function counters(values: number[], stepMs = SAMPLE_INTERVAL_MS): CounterSample[] {
  const end = Date.now();
  return values.map((v, i) => ({
    t: end - (values.length - 1 - i) * stepMs,
    rx_bytes: v,
    tx_bytes: v,
  }));
}

const rx = (s: CounterSample) => s.rx_bytes;

describe("counterRates", () => {
  /// The ordinary case: a counter climbing steadily is a steady rate.
  it("differences a rising counter into bytes per second", () => {
    // 60 kB per 60-second interval = 1000 B/s.
    const points = counterRates(counters([0, 60_000, 120_000, 180_000]), rx);
    expect(points).toHaveLength(3);
    for (const p of points) expect(p.v).toBeCloseTo(1000, 5);
  });

  /// A rate belongs to an INTERVAL, so N samples give N-1 points, and
  /// each one is stamped where the traffic was measured rather than
  /// where the interval began.
  it("stamps each rate with the newer sample's time", () => {
    const samples = counters([0, 1000, 2000]);
    const points = counterRates(samples, rx);
    expect(points).toHaveLength(2);
    expect(points[0].t).toBe(samples[1].t);
    expect(points[1].t).toBe(samples[2].t);
  });

  /// **Mutation test for #719's central requirement.**
  ///
  /// An interface that goes down, or a machine that reboots, restarts
  /// its byte count at zero. The naive difference is a large NEGATIVE
  /// number, and the tempting fix -- clamping to zero -- renders a
  /// reboot as an idle minute, which is a claim about the traffic
  /// rather than an admission that the counter is no longer comparable.
  ///
  /// If this fails, a counter reset is being drawn as a plausible rate.
  it("renders a counter reset as a gap, not as a rate", () => {
    // Four gigabytes accumulated, then a reboot back to near zero.
    const points = counterRates(counters([4_000_000_000, 4_000_060_000, 12_000, 72_000]), rx);
    expect(points).toHaveLength(3);
    expect(points[0].v).toBeCloseTo(1000, 5);
    // The reset interval.
    expect(points[1].v).toBeNull();
    expect(points[1].reason).toBe("reset");
    // And counting resumes normally afterwards, from the new baseline.
    expect(points[2].v).toBeCloseTo(1000, 5);
  });

  /// The reset must never become a zero, which is the specific wrong
  /// fix. Stated separately from the test above because "is null" and
  /// "is not zero" fail differently: a clamp would pass a `toHaveLength`
  /// check and only this assertion catches it.
  it("never clamps a reset to a plausible zero", () => {
    const points = counterRates(counters([9_000_000_000, 0]), rx);
    expect(points[0].v).not.toBe(0);
    expect(points[0].v).toBeNull();
  });

  /// A reset point is `null`, and `splitOnGaps` already breaks a run on
  /// a null value -- so the chart draws the reset as a gap with no
  /// reset-specific code in the component at all. This is the test that
  /// the two pieces actually compose.
  it("hands splitOnGaps something it already knows how to break", () => {
    const points = counterRates(
      counters([1_000_000, 1_060_000, 5, 60_005, 120_005]),
      rx,
    );
    const runs = splitOnGaps(points);
    expect(runs).toHaveLength(2);
    expect(runs[0]).toHaveLength(1);
    expect(runs[1]).toHaveLength(2);
  });

  /// A few bytes backwards is a mid-update reading, not a reboot.
  /// Treating it as a reset would pepper the chart with breaks that
  /// mean nothing, which costs the real breaks their meaning.
  it("treats a handful of bytes backwards as noise", () => {
    const points = counterRates(counters([1_000_000, 999_999]), rx);
    expect(points[0].v).toBe(0);
    expect(points[0].reason).toBeUndefined();
  });

  /// **Mutation test for the gap half.**
  ///
  /// Two samples either side of a closed laptop are hours apart, and
  /// the counter climbed by a whole day of traffic. Dividing by the
  /// elapsed time gives an arithmetically correct but meaningless
  /// average over a period nobody watched -- and, crucially, the point
  /// it produces must be one `splitOnGaps` cuts away rather than one
  /// the chart joins into a line.
  it("does not draw a line across a period nobody measured", () => {
    const end = Date.now();
    const samples: CounterSample[] = [
      { t: end - 12 * 3600_000 - 60_000, rx_bytes: 0, tx_bytes: 0 },
      { t: end - 12 * 3600_000, rx_bytes: 60_000, tx_bytes: 60_000 },
      // Twelve hours closed; the counter kept climbing all night.
      { t: end - 60_000, rx_bytes: 40_000_000_000, tx_bytes: 40_000_000_000 },
      { t: end, rx_bytes: 40_000_060_000, tx_bytes: 40_000_060_000 },
    ];
    const points = counterRates(samples, rx);
    // Three intervals: one measured minute, then the twelve-hour
    // stretch, then one more measured minute.
    expect(points).toHaveLength(3);
    // The overnight interval DOES produce an arithmetic value -- 40 GB
    // over twelve hours is a real division. What must not happen is the
    // chart drawing a LINE through it, and that is `splitOnGaps`'s job:
    // the point lands twelve hours after its predecessor, so the run is
    // cut before it.
    const runs = splitOnGaps(points);
    expect(runs).toHaveLength(2);
    // The first measured minute, alone -- never joined across the night.
    expect(runs[0]).toHaveLength(1);
    expect(runs[0][0].t).toBe(points[0].t);
    // The overnight point begins the second run rather than extending
    // the first, so nothing is drawn spanning the twelve hours.
    expect(runs[1][0].t).toBe(points[1].t);
    // And no polyline anywhere spans the gap.
    for (const run of runs) {
      for (let i = 1; i < run.length; i += 1) {
        expect(run[i].t - run[i - 1].t).toBeLessThan(30 * 60_000);
      }
    }
  });

  /// Two rows sharing an instant would divide by zero and produce an
  /// Infinity, which draws as a spike off the top of the chart.
  it("refuses to divide by a zero interval", () => {
    const t = Date.now();
    const points = counterRates(
      [
        { t, rx_bytes: 0, tx_bytes: 0 },
        { t, rx_bytes: 5000, tx_bytes: 5000 },
      ],
      rx,
    );
    expect(points[0].v).toBeNull();
    expect(points[0].reason).toBe("gap");
  });

  /// One sample is no interval, so there is nothing to say.
  it("gives no rate for a single sample", () => {
    expect(counterRates(counters([1000]), rx)).toEqual([]);
    expect(counterRates([], rx)).toEqual([]);
  });
});

describe("interfaceRates", () => {
  const at = (minsAgo: number) => new Date(Date.now() - minsAgo * 60_000).toISOString();

  /// Each interface is differenced against its OWN history, not against
  /// whatever happened to sit beside it in the sample.
  it("keeps interfaces separate", () => {
    const rates = interfaceRates([
      {
        sampled_at: at(2),
        networks: [
          { name: "en0", rx_bytes: 0, tx_bytes: 0 },
          { name: "utun0", rx_bytes: 500_000, tx_bytes: 0 },
        ],
      },
      {
        sampled_at: at(1),
        networks: [
          { name: "en0", rx_bytes: 60_000, tx_bytes: 0 },
          { name: "utun0", rx_bytes: 500_000, tx_bytes: 0 },
        ],
      },
    ]);
    expect(rates.get("en0")?.rx[0].v).toBeCloseTo(1000, 5);
    // The VPN moved nothing, which is a real zero -- it was measured.
    expect(rates.get("utun0")?.rx[0].v).toBe(0);
  });

  /// An interface that appears partway through has no points before it
  /// existed. Not a zero: a VPN that was down was not an idle VPN, it
  /// was not there.
  it("gives an interface no history from before it existed", () => {
    const rates = interfaceRates([
      { sampled_at: at(3), networks: [{ name: "en0", rx_bytes: 0, tx_bytes: 0 }] },
      {
        sampled_at: at(2),
        networks: [
          { name: "en0", rx_bytes: 60_000, tx_bytes: 0 },
          { name: "utun0", rx_bytes: 0, tx_bytes: 0 },
        ],
      },
      {
        sampled_at: at(1),
        networks: [
          { name: "en0", rx_bytes: 120_000, tx_bytes: 0 },
          { name: "utun0", rx_bytes: 60_000, tx_bytes: 0 },
        ],
      },
    ]);
    expect(rates.get("en0")?.rx).toHaveLength(2);
    expect(rates.get("utun0")?.rx).toHaveLength(1);
  });

  /// An interface that DISAPPEARS mid-series must not have its two
  /// surviving readings differenced across the stretch it was absent.
  /// That is the same unmeasured-period error as a gap, and the point
  /// it produces has to be one `splitOnGaps` cuts.
  it("does not bridge the stretch an interface was missing", () => {
    const rates = interfaceRates([
      {
        sampled_at: new Date(Date.now() - 120 * 60_000).toISOString(),
        networks: [{ name: "utun0", rx_bytes: 1_000_000, tx_bytes: 0 }],
      },
      // An hour with no utun0 at all.
      {
        sampled_at: at(1),
        networks: [{ name: "utun0", rx_bytes: 2_000_000, tx_bytes: 0 }],
      },
    ]);
    const points = rates.get("utun0")?.rx ?? [];
    expect(points).toHaveLength(1);
    // The single point sits two hours after nothing, so the chart shows
    // one dot rather than a line claiming an hour of steady traffic.
    expect(splitOnGaps(points)).toHaveLength(1);
    expect(splitOnGaps(points)[0]).toHaveLength(1);
  });

  it("ignores a sample with an unparseable timestamp", () => {
    const rates = interfaceRates([
      { sampled_at: "not a date", networks: [{ name: "en0", rx_bytes: 0, tx_bytes: 0 }] },
      { sampled_at: at(1), networks: [{ name: "en0", rx_bytes: 60_000, tx_bytes: 0 }] },
    ]);
    expect(rates.get("en0")?.rx).toHaveLength(0);
  });

  it("is empty for an empty series", () => {
    expect(interfaceRates([]).size).toBe(0);
  });
});

describe("formatRate", () => {
  /// Decimal, not binary: throughput is quoted in decimal everywhere a
  /// user would compare this against.
  it("scales through decimal units and says they are rates", () => {
    expect(formatRate(500)).toBe("500 B/s");
    expect(formatRate(1000)).toBe("1.0 kB/s");
    expect(formatRate(4_200_000)).toBe("4.2 MB/s");
  });

  /// The unit is the point. A throughput rendered with a size helper
  /// reads as a quantity, and the two are acted on very differently.
  it("never renders a bare size", () => {
    expect(formatRate(4_200_000)).toContain("/s");
  });
});

describe("peakRate", () => {
  it("ignores the points that were never measured", () => {
    expect(
      peakRate([
        { t: 1, v: 100 },
        { t: 2, v: null, reason: "reset" },
        { t: 3, v: 900 },
      ]),
    ).toBe(900);
  });

  /// Nothing measured is `null`, not 0 -- a chart scaled to a zero
  /// ceiling would draw every point on the axis and look like a flat
  /// line at no traffic.
  it("is null when nothing was measured at all", () => {
    expect(peakRate([{ t: 1, v: null, reason: "gap" }])).toBeNull();
    expect(peakRate([])).toBeNull();
  });
});
