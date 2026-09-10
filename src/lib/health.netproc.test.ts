/// Turning two readings of the per-process network table into rates
/// (#718).
///
/// Its own file rather than more cases in `health.rates.test.ts`: that
/// file is about differencing ONE counter over a series, and this is
/// about MATCHING processes across two readings before anything can be
/// differenced at all. The matching is where the interesting failures
/// are, and they are pure arithmetic over identities -- no DOM needed.
///
/// # Every process name below is invented
///
/// This repository is public. A real process list names what a person
/// runs, so these fixtures use synthetic names (`acme-sync`,
/// `widget-daemon`) rather than a capture from any machine. Per
/// `CONTRIBUTING.md`.

import { describe, expect, it } from "vitest";
import { netProcessRates, type NetProcessReading } from "./health";

/// Two readings fifteen seconds apart -- the Network page's cadence.
const STEP_MS = 15_000;

function reading(
  processes: NetProcessReading["processes"],
  t = 0,
): NetProcessReading {
  return { t, processes };
}

describe("netProcessRates", () => {
  it("differences two readings into bytes per second", () => {
    const rates = netProcessRates(
      reading([{ name: "acme-sync", pid: 501, bytes_in: 1_000, bytes_out: 500 }], 0),
      reading(
        [{ name: "acme-sync", pid: 501, bytes_in: 151_000, bytes_out: 30_500 }],
        STEP_MS,
      ),
    );
    expect(rates).toHaveLength(1);
    // 150 kB over 15 s is 10 kB/s in; 30 kB over 15 s is 2 kB/s out.
    expect(rates[0].in_rate).toBe(10_000);
    expect(rates[0].out_rate).toBe(2_000);
    // The cumulative totals ride along: "what has this moved
    // altogether" is a real question a rate cannot answer.
    expect(rates[0].bytes_in).toBe(151_000);
    expect(rates[0].bytes_out).toBe(30_500);
    expect(rates[0].pid).toBe(501);
  });

  /// **A process is matched by PID, never by name where a PID exists.**
  ///
  /// Several processes of one application share a name -- a browser is
  /// five or six -- so matching on the name would difference one
  /// helper's counters against another's and produce a rate that
  /// belongs to neither. Here two processes share a name and have
  /// moved completely different amounts.
  it("matches by pid, so two processes of one name do not cross", () => {
    const rates = netProcessRates(
      reading(
        [
          { name: "acme-helper", pid: 100, bytes_in: 0, bytes_out: 0 },
          { name: "acme-helper", pid: 200, bytes_in: 1_000_000, bytes_out: 0 },
        ],
        0,
      ),
      reading(
        [
          { name: "acme-helper", pid: 100, bytes_in: 15_000, bytes_out: 0 },
          { name: "acme-helper", pid: 200, bytes_in: 1_000_000, bytes_out: 0 },
        ],
        STEP_MS,
      ),
    );
    const byPid = new Map(rates.map((r) => [r.pid, r]));
    expect(byPid.get(100)?.in_rate).toBe(1_000);
    // The second one moved nothing across the interval, and that is a
    // real zero -- it was measured twice and did not advance.
    expect(byPid.get(200)?.in_rate).toBe(0);
  });

  /// A process present only in the NEWER reading has no interval, so it
  /// gets no rate. Its cumulative total is not a delta from zero: it
  /// may have been running for hours and simply absent from the earlier
  /// table.
  it("gives a newly appeared process no rate rather than its lifetime total", () => {
    const rates = netProcessRates(
      reading([{ name: "acme-sync", pid: 1, bytes_in: 0, bytes_out: 0 }], 0),
      reading(
        [
          { name: "acme-sync", pid: 1, bytes_in: 0, bytes_out: 0 },
          // 6 GB of lifetime traffic, first seen now.
          { name: "widget-daemon", pid: 2, bytes_in: 6e9, bytes_out: 0 },
        ],
        STEP_MS,
      ),
    );
    expect(rates.map((r) => r.name)).toEqual(["acme-sync"]);
    expect(rates.some((r) => r.in_rate > 1e6)).toBe(false);
  });

  /// A process present only in the OLDER reading has exited. There is
  /// nothing to difference, and reporting its last total would
  /// attribute a week of traffic to fifteen seconds.
  it("drops a process that exited between the readings", () => {
    const rates = netProcessRates(
      reading(
        [
          { name: "acme-sync", pid: 1, bytes_in: 0, bytes_out: 0 },
          { name: "widget-daemon", pid: 2, bytes_in: 6e9, bytes_out: 0 },
        ],
        0,
      ),
      reading([{ name: "acme-sync", pid: 1, bytes_in: 0, bytes_out: 0 }], STEP_MS),
    );
    expect(rates.map((r) => r.name)).toEqual(["acme-sync"]);
  });

  /// **A counter that went backwards produces no rate.**
  ///
  /// The realistic cause is a recycled PID: the process that held it
  /// exited and a new one took the number, so the two readings are of
  /// two different programs. Clamping to zero would claim a
  /// measurement of an idle process that is not the one being watched.
  it("drops a backwards counter rather than clamping it to zero", () => {
    const rates = netProcessRates(
      reading([{ name: "acme-sync", pid: 501, bytes_in: 5_000_000, bytes_out: 0 }], 0),
      reading(
        [{ name: "acme-sync", pid: 501, bytes_in: 1_000, bytes_out: 0 }],
        STEP_MS,
      ),
    );
    expect(rates).toEqual([]);
  });

  /// A row with no PID falls back to matching by name -- but only
  /// against another row with no PID. A process named `42` must never
  /// be matched against PID 42, which is what the key prefixes prevent.
  it("keeps the pid and name key spaces apart", () => {
    const rates = netProcessRates(
      reading(
        [
          { name: "42", pid: null, bytes_in: 0, bytes_out: 0 },
          { name: "acme-sync", pid: 42, bytes_in: 1_000_000, bytes_out: 0 },
        ],
        0,
      ),
      reading(
        [
          { name: "42", pid: null, bytes_in: 15_000, bytes_out: 0 },
          { name: "acme-sync", pid: 42, bytes_in: 1_000_000, bytes_out: 0 },
        ],
        STEP_MS,
      ),
    );
    const named = rates.find((r) => r.pid === null);
    expect(named?.in_rate).toBe(1_000);
    expect(rates.find((r) => r.pid === 42)?.in_rate).toBe(0);
  });

  /// A non-positive interval yields nothing rather than `Infinity`,
  /// which would render as a spike off the top of any figure. Two
  /// readings sharing an instant is the ordinary way this happens --
  /// a re-render handed the same data twice.
  it("refuses to divide by a zero or negative interval", () => {
    const rows = [{ name: "acme-sync", pid: 1, bytes_in: 0, bytes_out: 0 }];
    const later = [{ name: "acme-sync", pid: 1, bytes_in: 1_000, bytes_out: 0 }];
    expect(netProcessRates(reading(rows, 5_000), reading(later, 5_000))).toEqual([]);
    expect(netProcessRates(reading(rows, 5_000), reading(later, 1_000))).toEqual([]);
  });
});
