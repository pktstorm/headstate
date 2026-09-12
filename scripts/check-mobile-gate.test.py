#!/usr/bin/env python3
"""Proof that the mobile-gate guard can fail, and on what.

"A guard nobody has watched fail is a guard that might be checking
nothing" -- `check-symlinks.test.py`, and it applies verbatim. This guard
was the one of four in `lint-deps` with no self-test (#853), which is an
uncomfortable place for it to be: the bug it hunts is itself a silent
pass, so a guard that had quietly stopped checking would look exactly
like the green board it exists to prevent.

Two things are pinned, and they are the two that can rot:

1. `gated_jobs()` -- the derivation that replaced a hardcoded
   `GATED_JOBS = ("mobile-android", "mobile-ios")`. A third gated job
   added later was previously not checked at all while the guard printed
   success. The cases below include that exact scenario.

2. The step-gating walk over a synthetic workflow, driven through
   `jobs_of`/`steps_of`, including the catastrophic `needs`-dropped case
   from the guard's own docstring.

Synthetic YAML throughout rather than mutating the real `ci.yml`: the
real file is what the guard is pointed at in `lint-deps`, and a test that
rewrote it would be testing a file it had just broken. The fixtures are
shaped like `ci.yml` (jobs at 2 spaces, steps at `      - `) because that
indentation IS the parser's contract -- the guard reads YAML by
indentation rather than importing PyYAML, so the shape is the interface.

Run: python3 scripts/check-mobile-gate.test.py
"""

import importlib.util
import pathlib
import sys

HERE = pathlib.Path(__file__).resolve().parent
spec = importlib.util.spec_from_file_location("gate_guard", HERE / "check-mobile-gate.py")
guard = importlib.util.module_from_spec(spec)
spec.loader.exec_module(guard)

failures: list[str] = []

GATE = guard.GATE_JOB
COND = guard.CONDITION


def derives(name: str, yaml: str, expected: list[str]) -> None:
    got = guard.gated_jobs(guard.jobs_of(yaml))
    if got != expected:
        failures.append(f"{name}\n    derived {got}, expected {expected}")


# ---- gated_jobs(): the derivation that replaced the hardcoded tuple ----

# The shape `ci.yml` actually has: a scalar `needs:`.
derives(
    "the scalar `needs:` form, as ci.yml spells it",
    f"""
jobs:
  {GATE}:
    runs-on: ubuntu-latest
  mobile-ios:
    needs: {GATE}
    runs-on: macos-latest
  lint:
    runs-on: macos-latest
""",
    ["mobile-ios"],
)

# The regression this derivation exists for: a THIRD gated job. Under the
# hardcoded tuple this job was not checked at all and the guard still
# printed success, so an ungated step in it was invisible (#853).
derives(
    "a third gated job is found without being listed anywhere",
    f"""
jobs:
  {GATE}:
    runs-on: ubuntu-latest
  mobile-android:
    needs: {GATE}
    runs-on: ubuntu-latest
  mobile-ios:
    needs: {GATE}
    runs-on: macos-latest
  mobile-windows:
    needs: {GATE}
    runs-on: windows-latest
""",
    ["mobile-android", "mobile-ios", "mobile-windows"],
)

# A job gaining a SECOND dependency must stay in the checked set. Written
# because the naive `needs: mobile-changes` string match drops it, which
# would be the hardcoding bug in a new costume -- the job is still gated,
# the guard just stops looking at it.
derives(
    "the flow-list `needs: [a, b]` form keeps the job in the set",
    f"""
jobs:
  {GATE}:
    runs-on: ubuntu-latest
  mobile-ios:
    needs: [lint, {GATE}]
    runs-on: macos-latest
""",
    ["mobile-ios"],
)

derives(
    "the block-list `needs:` form keeps the job in the set",
    f"""
jobs:
  {GATE}:
    runs-on: ubuntu-latest
  mobile-ios:
    needs:
      - lint
      - {GATE}
    runs-on: macos-latest
""",
    ["mobile-ios"],
)

# The catastrophic case, from the guard's docstring: a job that loses its
# `needs` drops OUT of the derived set (which is why KNOWN_GATED exists as
# a floor -- asserted in the main() cases below, not here).
derives(
    "a job that dropped `needs` is not derived as gated",
    f"""
jobs:
  {GATE}:
    runs-on: ubuntu-latest
  mobile-ios:
    runs-on: macos-latest
""",
    [],
)

# The gate job itself depends on nothing and must never be checked as one
# of its own dependents.
derives(
    "the gate job is never treated as gated by itself",
    f"""
jobs:
  {GATE}:
    runs-on: ubuntu-latest
  other:
    needs: lint
    runs-on: macos-latest
""",
    [],
)

# A job depending on something else entirely is not in scope.
derives(
    "a job gated on a different dependency is not in scope",
    f"""
jobs:
  {GATE}:
    runs-on: ubuntu-latest
  build:
    needs: lint
    runs-on: macos-latest
""",
    [],
)


# ---- main(): the end-to-end verdict over a synthetic workflow ----
#
# `main()` reads a module-level path, so it is pointed at a temp file.
# That is the whole reason these are separate from the cases above: the
# derivation takes text and is pure, the verdict takes a file.

import tempfile


def verdict(name: str, yaml: str, should_pass: bool) -> None:
    with tempfile.TemporaryDirectory() as d:
        path = pathlib.Path(d) / "ci.yml"
        path.write_text(yaml)
        original = guard.WORKFLOW
        guard.WORKFLOW = path
        try:
            # The guard prints its findings; a self-test that dumped them
            # all would bury its own result.
            out = sys.stdout
            sys.stdout = open("/dev/null", "w")
            try:
                code = guard.main()
            finally:
                sys.stdout.close()
                sys.stdout = out
        finally:
            guard.WORKFLOW = original
    passed = code == 0
    if passed != should_pass:
        want = "pass" if should_pass else "fail"
        failures.append(f"{name}\n    expected the guard to {want}, it returned {code}")


# Both known jobs present, gated, every step carrying the condition.
GOOD = f"""
jobs:
  {GATE}:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v5
  mobile-android:
    needs: {GATE}
    runs-on: ubuntu-latest
    steps:
      - name: Checkout
        if: {COND}
        uses: actions/checkout@v5
      - name: Build
        if: {COND}
        run: make check-mobile-android
  mobile-ios:
    needs: {GATE}
    runs-on: macos-latest
    steps:
      - name: Checkout
        if: {COND}
        uses: actions/checkout@v5
"""

verdict("a correctly wired workflow passes", GOOD, True)

# The failure this guard was written for. `needs` gone: every step's
# condition evaluates to the empty string, so the job reports green
# having compiled nothing. Caught by the KNOWN_GATED floor, since the
# derivation no longer sees the job at all.
verdict(
    "a job that lost `needs: mobile-changes` fails",
    GOOD.replace(f"    needs: {GATE}\n    runs-on: macos-latest", "    runs-on: macos-latest"),
    False,
)

# The mirror-image failure: a step added later without the gate.
verdict(
    "an ungated step fails",
    GOOD.replace(
        """      - name: Build
        if: {cond}
        run: make check-mobile-android""".format(cond=COND),
        """      - name: Build
        run: make check-mobile-android""",
    ),
    False,
)

# The gate job itself deleted: the jobs have nothing to read their
# condition from.
verdict(
    "a workflow with no mobile-changes job fails",
    GOOD.replace(f"  {GATE}:\n    runs-on: ubuntu-latest\n    steps:\n      - uses: actions/checkout@v5\n", ""),
    False,
)

# A known job deleted outright, as opposed to un-gated.
verdict(
    "a missing known job fails",
    GOOD.replace(
        f"""  mobile-ios:
    needs: {GATE}
    runs-on: macos-latest
    steps:
      - name: Checkout
        if: {COND}
        uses: actions/checkout@v5
""",
        "",
    ),
    False,
)

# A third gated job whose steps are ungated. This is the case the
# hardcoded tuple could not see: the job was not in GATED_JOBS, so it was
# never walked, and the guard printed success.
verdict(
    "a third gated job with an ungated step fails",
    GOOD
    + f"""  mobile-windows:
    needs: {GATE}
    runs-on: windows-latest
    steps:
      - name: Build
        run: make check-mobile-windows
""",
    False,
)

# And the same third job, correctly gated, still passes -- so the case
# above is failing for the ungated step and not merely for existing.
verdict(
    "a third gated job that is correctly gated passes",
    GOOD
    + f"""  mobile-windows:
    needs: {GATE}
    runs-on: windows-latest
    steps:
      - name: Build
        if: {COND}
        run: make check-mobile-windows
""",
    True,
)


if failures:
    print("check-mobile-gate.py self-test FAILED:")
    for f in failures:
        print(f"  {f}")
    sys.exit(1)

print("check-mobile-gate.py self-test: the gate guard rejects what it should.")
