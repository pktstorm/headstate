#!/usr/bin/env bash
# Headstate is a public repo. At runtime it queries every org the user can
# see -- that is the product. But committed files must never name a private
# or employer repository, because a leak here is permanent and public.
#
# This is an ALLOW-list, deliberately. A deny-list would have to spell out
# the very names it exists to keep out, putting them in the repo in plain
# text and defeating itself. An allow-list also catches owners nobody
# thought to enumerate.
#
# Two patterns are scanned, both anchored so they cannot match ordinary
# prose. An earlier unanchored `owner/repo` pattern matched things like
# `api/hooks.ts`, `read/write`, and `5000/hour` -- a gate that cries wolf
# gets disabled, so anchoring is load-bearing, not tidiness.
#   1. github.com/<owner>/<repo>   -- any GitHub URL
#   2. <owner>/<repo>#<number>     -- the PR/issue shorthand
set -euo pipefail

# The only repository owners this project legitimately references.
# `org` and `owner` are the generic placeholders used in format examples
# (`- [org/repo#123] Title`), not real accounts.
ALLOWED='octocat|pktstorm|tauri-apps|shadcn-ui|rust-lang|actions|dtolnay|Swatinem|org|owner'

# Lockfiles are machine-generated dependency graphs naming hundreds of
# upstream repos; they are not a leak vector for private names.
EXCLUDES=(':!scripts/check-privacy.sh' ':!yarn.lock' ':!src-tauri/Cargo.lock')

urls=$(git grep -hoIE 'github\.com/[A-Za-z0-9][-A-Za-z0-9_.]*/[A-Za-z0-9][-A-Za-z0-9_.]+' \
         -- "${EXCLUDES[@]}" 2>/dev/null \
       | sed -E 's#.*github\.com/##' || true)

refs=$(git grep -hoIE '[A-Za-z0-9][-A-Za-z0-9_.]*/[A-Za-z0-9][-A-Za-z0-9_.]+#[0-9]+' \
         -- "${EXCLUDES[@]}" 2>/dev/null \
       | sed -E 's/#[0-9]+$//' || true)

found=$(printf '%s\n%s\n' "$urls" "$refs" \
        | grep -vE '^[[:space:]]*$' \
        | grep -vE "^($ALLOWED)/" \
        | sort -u || true)

if [ -n "$found" ]; then
  echo "ERROR: repository references with a non-allow-listed owner:"
  echo "$found" | sed 's/^/  /'
  echo
  echo "Use synthetic fixtures (octocat/hello-world), or add the owner to"
  echo "ALLOWED in this script if it is a legitimate public dependency."
  exit 1
fi

echo "privacy check: clean"
