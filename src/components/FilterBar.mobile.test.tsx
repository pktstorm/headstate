import { fireEvent, render, screen } from "@testing-library/react";
import { afterEach, describe, expect, it } from "vitest";
import { stubViewport } from "@/test-utils";
import { activeFilterCount } from "@/lib/derive";
import { FilterBar } from "./FilterBar";

/// The bar holds nine controls. On a desktop that is one `flex-wrap`
/// row; at 390px, minus `px-4`, there are 358px of usable width and the
/// search input alone claimed 256px of it -- so the bar wrapped to five
/// or six rows of chips, permanently, above the list on the app's
/// primary screen.

afterEach(() => {
  stubViewport(null);
});

function renderBar() {
  return render(<FilterBar prs={[]} />);
}

describe("FilterBar on a phone", () => {
  it("puts the eight controls behind a Filters button", async () => {
    stubViewport(390);
    renderBar();
    // The search stays: it is the control people reach for most, and
    // hiding it behind a button would cost a tap on every use.
    expect(screen.getByLabelText(/search pull requests/i)).toBeTruthy();
    expect(screen.getByRole("button", { name: /^filters/i })).toBeTruthy();
    // Everything else is behind it.
    expect(screen.queryByRole("button", { name: /clear filters/i })).toBeNull();

    fireEvent.click(screen.getByRole("button", { name: /^filters/i }));
    expect(await screen.findByRole("button", { name: /clear filters/i })).toBeTruthy();
  });

  it("keeps every control inline on a desktop", () => {
    stubViewport(1400);
    renderBar();
    // No Filters button, and the controls are there without a tap.
    expect(screen.queryByRole("button", { name: /^filters/i })).toBeNull();
    expect(screen.getByRole("button", { name: /clear filters/i })).toBeTruthy();
  });
});

describe("activeFilterCount", () => {
  it("counts nothing for an empty filter set", () => {
    expect(activeFilterCount({})).toBe(0);
  });

  it("ignores the search and the sort", () => {
    // The search field sits visibly beside the button and speaks for
    // itself; sorting hides nothing. Counting either would make the
    // badge argue with what the user can already see.
    expect(activeFilterCount({ query: "auth", sort: "newest" })).toBe(0);
  });

  it("counts set flags and non-empty label lists", () => {
    expect(
      activeFilterCount({
        draftsOnly: true,
        includeLabels: ["bug"],
        ci: "failure",
      }),
    ).toBe(3);
  });

  it("does not count a flag that is off or a list that is empty", () => {
    // `false` and `[]` are what a cleared control leaves behind, and a
    // badge reading "(2)" over a bar that filters nothing is worse than
    // no badge.
    expect(activeFilterCount({ draftsOnly: false, includeLabels: [] })).toBe(0);
  });
});
