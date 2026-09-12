import { render, screen } from "@testing-library/react";
import { describe, expect, it } from "vitest";
import type { StatsOutcome } from "@/types/pr";
import { ScopeCounts } from "./ScopeSummary";

/// A complete, uncapped answer. Fields are spelled out rather than
/// defaulted so a test that cares about one of them says so by overriding
/// it, and so a field added to `StatsOutcome` is a compile error here rather
/// than a silent `undefined` in every case below.
const outcome = (over: Partial<StatsOutcome> = {}): StatsOutcome => ({
  total: 706,
  retrievable: true,
  unretrievable: 0,
  slices: 1,
  rounds: 1,
  viaConnection: true,
  // `remaining: null` rather than a number: null means nothing reported a
  // budget, which `Spend` documents as NOT the same as zero, and a fixture
  // should not assert a measurement it does not have.
  spend: { points: 2, requests: 2, unmetered: 0, remaining: null, resetAt: null },
  refusedFields: 0,
  ...over,
});

describe("ScopeCounts", () => {
  /// #851: `viaConnection` existed so "a reader can tell WHICH completeness
  /// guarantee they have" and no reader read it.
  ///
  /// The failure it leaves is not a wrong number -- both totals are exact --
  /// but two figures that look identical while resting on different
  /// arguments. A connection total is uncapped by construction; a search
  /// total is assembled from slices each capped at 1,000 results, and its
  /// completeness rests on the slicing having covered the window.
  ///
  /// Asserted on BOTH values of the flag, because a hint that happened to
  /// mention one source unconditionally would pass a single-sided check
  /// while still telling every reader the same thing.
  it("says which source a total came from", () => {
    const { unmount } = render(
      <ScopeCounts merged={outcome()} opened={outcome()} days={30} failed={0} />,
    );
    expect(screen.getAllByText(/uncapped connection/).length).toBeGreaterThan(0);
    expect(screen.queryByText(/sliced search/)).toBeNull();
    unmount();

    render(
      <ScopeCounts
        merged={outcome({ viaConnection: false })}
        opened={outcome({ viaConnection: false })}
        days={30}
        failed={0}
      />,
    );
    expect(screen.getAllByText(/sliced search/).length).toBeGreaterThan(0);
    expect(screen.queryByText(/uncapped connection/)).toBeNull();
  });

  /// The distinction must not depend on `slices > 1`.
  ///
  /// This is the specific inference #851 called "neither necessary nor
  /// sufficient", and it is the reason the flag had to be rendered rather
  /// than left to be deduced: a `search` answered in ONE slice is still
  /// capped, so before this change it was indistinguishable on screen from
  /// an uncapped connection total.
  it("distinguishes a single-slice search from a connection", () => {
    const { unmount } = render(
      <ScopeCounts
        merged={outcome({ viaConnection: false, slices: 1 })}
        opened={outcome({ viaConnection: false, slices: 1 })}
        days={30}
        failed={0}
      />,
    );
    expect(screen.getAllByText(/sliced search/).length).toBeGreaterThan(0);
    // And says nothing about assembly, which genuinely did not happen.
    expect(screen.queryByText(/assembled from/)).toBeNull();
    unmount();

    render(
      <ScopeCounts
        merged={outcome({ viaConnection: true, slices: 1 })}
        opened={outcome({ viaConnection: true, slices: 1 })}
        days={30}
        failed={0}
      />,
    );
    expect(screen.getAllByText(/uncapped connection/).length).toBeGreaterThan(0);
  });

  /// A failed count renders "--", never a 0.
  ///
  /// Not new behaviour -- `ScopeCounts`' own doc records it as #826's
  /// requirement -- but it had no test, and it is the property the source
  /// label above must not have broken: an absent outcome has no
  /// `viaConnection` to report, so the new line must be absent too rather
  /// than rendering a guarantee about a number that does not exist.
  it("shows an unmeasured count as absent, with no source claim", () => {
    render(
      <ScopeCounts merged={undefined} opened={outcome()} days={30} failed={1} />,
    );
    expect(screen.getAllByText("--").length).toBe(1);
    expect(screen.getByText(/could not measure/)).toBeDefined();
    // Exactly one source claim: the opened figure's. The failed one must
    // not carry a completeness guarantee at all.
    expect(screen.getAllByText(/uncapped connection/).length).toBe(1);
  });
});
