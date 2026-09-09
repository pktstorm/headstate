import { describe, expect, it } from "vitest";

/// WCAG relative luminance, then the contrast ratio between two colours.
///
/// Written out rather than pulled from a library: it is eight lines, it
/// is the definition the standard gives, and a dependency added to
/// assert a constant is a dependency to keep current forever.
function luminance(hex: string): number {
  const h = hex.replace("#", "");
  const [r, g, b] = [0, 2, 4].map((i) => parseInt(h.slice(i, i + 2), 16) / 255);
  const f = (c: number) => (c <= 0.03928 ? c / 12.92 : ((c + 0.055) / 1.055) ** 2.4);
  return 0.2126 * f(r) + 0.7152 * f(g) + 0.0722 * f(b);
}

function ratio(a: string, b: string): number {
  const [la, lb] = [luminance(a), luminance(b)];
  return (Math.max(la, lb) + 0.05) / (Math.min(la, lb) + 0.05);
}

/// #693: the solid button hovers carried white text at 3.35:1, under
/// the 4.5:1 WCAG requires of body text.
///
/// The measurement that reframed the issue: the hover BACKGROUNDS pass
/// comfortably against the page (5.6:1), and the RESTING colours pass
/// against white (4.63:1). So hovering made the label harder to read
/// than not hovering -- the interaction degraded the thing it was
/// meant to emphasise.
describe("button hover contrast", () => {
  const WHITE = "#ffffff";
  const GROUND = "#0d1117";

  const HOVERS = {
    primary: "#1a7f37",
    destructive: "#c93c37",
    nav: "#316dca",
  };

  /// The failure this issue is about. These carry white labels.
  it("keeps white label text legible on every solid hover", () => {
    for (const [name, bg] of Object.entries(HOVERS)) {
      expect(ratio(bg, WHITE), `${name} (${bg}) vs white`).toBeGreaterThanOrEqual(4.5);
    }
  });

  /// The constraint that made this a trade-off rather than a lookup:
  /// every colour passing with white text is DARKER than the resting
  /// one, so hover dims rather than brightens. It must still be
  /// clearly distinguishable, or the fix removes the affordance it was
  /// protecting.
  it("stays visibly distinct from the page behind it", () => {
    for (const [name, bg] of Object.entries(HOVERS)) {
      expect(ratio(bg, GROUND), `${name} (${bg}) vs the page`).toBeGreaterThanOrEqual(3);
    }
  });

  /// Guards the exact regression: the old values are recorded here so
  /// reintroducing one fails rather than quietly restoring 3.35:1.
  it("rejects the colours this issue replaced", () => {
    for (const old of ["#2ea043", "#f85149", "#388bfd"]) {
      expect(ratio(old, WHITE), `${old} was the failing value`).toBeLessThan(4.5);
    }
  });
});
