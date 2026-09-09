#!/usr/bin/env bash
# Keyboard focus must stay visible: `src/index.css` needs a
# `:focus-visible` rule that actually paints a ring.
#
# The bug this guards against already happened (#690). `index.css` had
# `* { @apply border-border outline-ring/50 }`, which sets an outline
# COLOUR and nothing else -- no width, no style -- so the ring never
# painted. Tailwind's preflight had already removed the engine's own by
# then, leaving roughly 150 raw `<button>` elements, nearly every control
# outside `ui/`, with no focus indicator at all.
#
# Nothing in the test suite can catch that regressing:
#
#   - jsdom applies no stylesheets, so a rendered component's computed
#     outline is always empty.
#   - importing the CSS as text with `?raw` returns empty, because
#     `@tailwindcss/vite` claims `.css` files.
#   - the repo deliberately avoids `node:fs` in tests.
#
# So deleting three lines silently un-fixes every button in the app while
# the suite stays green -- the same shape as the original bug: a rule that
# looks present, does nothing, and nothing notices.
#
# Asserted against the SOURCE, not a built bundle (unlike
# check-toast-css.sh, whose failure exists only after Vite extracts
# Sonner's inlined styles). This rule is written by hand and survives the
# build unchanged, so the source is where the regression would land and
# checking it needs no build first.
#
# The check is deliberately about the outline's WIDTH, not the selector's
# presence. `:focus-visible { outline-color: ... }` alone is precisely the
# original bug, and it would satisfy a grep for the selector.
set -euo pipefail

css="src/index.css"
if [ ! -f "$css" ]; then
  echo "ERROR: $css not found -- run this from the repository root" >&2
  exit 2
fi

fail() {
  echo "ERROR: $1" >&2
  echo "" >&2
  echo "Keyboard focus would be INVISIBLE across the app: Tailwind's" >&2
  echo "preflight removes the engine's own focus ring, and nearly every" >&2
  echo "control outside src/components/ui/ is a raw <button> with no" >&2
  echo "focus-visible utilities of its own." >&2
  echo "" >&2
  echo "$css needs a rule of the shape:" >&2
  echo "" >&2
  echo "    :focus-visible {" >&2
  echo "      outline: 2px solid #58a6ff;" >&2
  echo "      outline-offset: 2px;" >&2
  echo "    }" >&2
  echo "" >&2
  echo "See scripts/check-focus-css.sh and issue #690 for why." >&2
  exit 1
}

# The rule body: everything between the `:focus-visible` selector and the
# closing brace. `sed` rather than a single grep because the declarations
# sit on their own lines, and the width and the selector must be shown to
# belong to the SAME rule -- a width somewhere else in the file is not a
# focus ring.
#
# `-n` with an explicit print: the range starts at the selector and ends
# at the first line holding `}`, which for this hand-written rule is its
# own closing brace.
rule=$(sed -n '/:focus-visible[^{]*{/,/}/p' "$css")

if [ -z "$rule" ]; then
  fail "$css has no ':focus-visible' rule at all"
fi

# A shorthand `outline:` counts only when it carries a length. `outline:
# none`, `outline: 0`, and a bare colour all parse fine and paint nothing.
#
# Accepted: `outline: 2px solid ...`, `outline: medium solid ...`, and the
# longhand `outline-width: 2px`. Rejected: a zero length in any unit, and
# the `thin`/`medium`/`thick` keywords are allowed since all three are
# non-zero.
has_width=0
if grep -qE 'outline(-width)?: *[^;}]*(\b(0*[1-9][0-9]*|[0-9]*\.[0-9]*[1-9][0-9]*)(px|r?em|pt|ch|ex|vh|vw|vmin|vmax|cm|mm|in|pc|q)\b|\b(thin|medium|thick)\b)' <<<"$rule"; then
  has_width=1
fi

if [ "$has_width" -eq 0 ]; then
  fail "the ':focus-visible' rule in $css declares no non-zero outline width"
fi

# A width with `outline-style: none` paints nothing either. The shorthand
# carries its own style (`2px solid ...`), so this only has to reject an
# explicit none/hidden longhand.
if grep -qE 'outline-style: *(none|hidden)' <<<"$rule"; then
  fail "the ':focus-visible' rule in $css sets 'outline-style: none'"
fi

echo "focus css check: clean"
