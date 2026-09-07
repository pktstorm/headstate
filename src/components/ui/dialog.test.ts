import { describe, expect, it } from "vitest";
import { cn } from "@/lib/utils";

/// The side margin every dialog relies on, and the merge behaviour that
/// used to silently delete it.
///
/// `DialogContent`'s base classes carried the margin as
/// `max-w-[calc(100%-2rem)]`. `cn` is `twMerge`, which treats `max-w-*`
/// as ONE conflict key -- so a caller passing `max-w-lg` did not narrow
/// the dialog within the cap, it removed the cap. All 18 call sites pass
/// one, so every dialog in the app lost it, and at 390px each rendered
/// exactly 390px wide: edge to edge, corners clipped by the screen, and
/// no backdrop strip left at the sides to tap.
///
/// This asserts the property rather than the string: the fix is that the
/// margin lives on a key the callers do not set, and a future refactor
/// that moves it back onto `max-w-*` should fail here rather than in
/// someone's hands.

/// The width classes `DialogContent` actually ships, kept in step with
/// `dialog.tsx` by the last test below.
const BASE = "w-[calc(100%-2rem)] sm:max-w-sm";

/// Every override passed by a real call site.
const CALLER_WIDTHS = ["max-w-lg", "max-w-2xl", "max-w-3xl sm:max-w-3xl"];

describe("the dialog's side margin", () => {
  it("survives every width a caller passes", () => {
    for (const caller of CALLER_WIDTHS) {
      expect(cn(BASE, caller)).toContain("w-[calc(100%-2rem)]");
    }
  });

  it("would NOT have survived on max-w, which is the bug", () => {
    // The shape of the old base. Kept as a test so the reason for the
    // `w-*` spelling is recorded as behaviour, not just a comment.
    const old = "w-full max-w-[calc(100%-2rem)] sm:max-w-sm";
    expect(cn(old, "max-w-lg")).not.toContain("max-w-[calc(100%-2rem)]");
  });

  it("still lets a caller set the dialog's maximum width", () => {
    // The margin must not have been fixed by pinning the width: a
    // desktop dialog that asked for `max-w-3xl` must still get it.
    expect(cn(BASE, "max-w-3xl")).toContain("max-w-3xl");
  });

  it("matches the classes dialog.tsx actually ships", async () => {
    // Guards the fixture above: a `BASE` that had drifted from the
    // component would make every assertion here meaningless.
    const source = await import("./dialog.tsx?raw");
    for (const cls of BASE.split(" ")) {
      expect(source.default).toContain(cls);
    }
  });
});
