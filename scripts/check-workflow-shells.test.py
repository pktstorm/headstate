#!/usr/bin/env python3
"""Proof that the workflow-shell guard can fail, and on what.

"A guard nobody has watched fail is a guard that might be checking
nothing" -- `check-symlinks.test.py`, and it applies here for the same
reason it applied to `check-mobile-gate.py`: this guard's whole purpose is
to catch a defect that is otherwise invisible until a Windows job parses
PowerShell, so a guard that had quietly stopped checking would look
exactly like the green board it exists to prevent.

#892 recorded this script as having a committed self-test. It did not --
this is that self-test, and it was written because adding one made the
missing case show up immediately.

THE CASE THAT MADE THIS NECESSARY, pinned below as
`a comment mentioning the label is not a windows job`: the platform test
was a bare `"windows-latest" in line`, which matched the label inside a
COMMENT. Any job whose comments merely mention windows-latest was then
treated as a Windows job, and every `run:` step after that comment was
reported. Writing one such comment in the macos-only `lint` job produced
eleven false positives at once. A gate that cries wolf gets disabled
(#853), so a guard that reports eleven phantom failures is worse than no
guard.

Synthetic YAML throughout rather than mutating the real `ci.yml`: the real
file is what the guard is pointed at in `lint-deps`, and a test that
rewrote it would be testing a file it had just broken. The fixtures are
shaped like `ci.yml` (jobs at 2 spaces, steps at `      - `) because that
indentation IS the parser's contract -- the guard reads YAML by
indentation rather than importing PyYAML, so the shape is the interface.

Run: python3 scripts/check-workflow-shells.test.py
"""

import importlib.util
import pathlib
import sys

HERE = pathlib.Path(__file__).resolve().parent
spec = importlib.util.spec_from_file_location(
    "shells_guard", HERE / "check-workflow-shells.py"
)
guard = importlib.util.module_from_spec(spec)
spec.loader.exec_module(guard)

failures: list[str] = []


def offenders(text: str) -> list[str]:
    """The steps this guard would report, by the same logic `main` uses.

    Driven through `steps_of` rather than through `main`, because `main`
    globs the real `.github/workflows` -- the directory these fixtures
    deliberately are not in.
    """
    import re

    bad = []
    for job_windows, step in guard.steps_of(text):
        body = "\n".join(step)
        if not re.search(r"^\s+run:", body, re.M):
            continue
        if re.search(r"^\s+shell:", body, re.M):
            continue
        if not job_windows:
            continue
        cond = re.search(r"^\s+if:.*$", body, re.M)
        if cond and any(o in cond.group(0) for o in guard.NON_WINDOWS):
            continue
        name = re.search(r"name:\s*(.+)$", body, re.M)
        bad.append(name.group(1).strip() if name else "(unnamed)")
    return bad


def verdict(label: str, text: str, expected: list[str]) -> None:
    got = offenders(text)
    if got != expected:
        failures.append(f"{label}: expected {expected}, got {got}")


HEAD = "name: probe\non: push\njobs:\n"

# The defect the guard exists for: a Windows-reachable `run:` with no
# `shell:`. This is the v2.0.1-rc.1 rehearsal failure in miniature.
verdict(
    "a windows run step with no shell is reported",
    HEAD
    + """  win:
    runs-on: windows-latest
    steps:
      - name: Stamp version from tag
        run: |
          set -euo pipefail
          echo "${GITHUB_REF#refs/tags/}"
""",
    ["Stamp version from tag"],
)

# The same step, declared. Proves the case above fails for the missing
# `shell:` and not merely for existing.
verdict(
    "the same step with shell: bash passes",
    HEAD
    + """  win:
    runs-on: windows-latest
    steps:
      - name: Stamp version from tag
        shell: bash
        run: |
          set -euo pipefail
""",
    [],
)

# A matrix job reaches Windows through `matrix.os`, not `runs-on`.
verdict(
    "a matrix including windows-latest counts as a windows job",
    HEAD
    + """  matrixed:
    strategy:
      matrix:
        os: [ubuntu-latest, windows-latest]
    runs-on: ${{ matrix.os }}
    steps:
      - name: Undeclared
        run: echo hi
""",
    ["Undeclared"],
)

# THE REGRESSION THIS FILE WAS WRITTEN FOR. The label appears only inside
# a comment, in a job that never runs on Windows. Before the `#`-strip
# this reported every step below the comment.
verdict(
    "a comment mentioning the label is not a windows job",
    HEAD
    + """  lint:
    runs-on: macos-latest
    steps:
      # Explaining what happens on a `windows-latest` job must not make
      # THIS job look like one.
      - name: First
        run: echo one
      - name: Second
        run: echo two
      - name: Third
        run: echo three
""",
    [],
)

# The comment-strip must not hide a real `runs-on`. Same job, same
# comment, but genuinely on Windows: every step is still reported.
verdict(
    "a comment does not mask a real windows runs-on",
    HEAD
    + """  win:
    runs-on: windows-latest
    steps:
      # A comment about `windows-latest` alongside the real thing.
      - name: First
        run: echo one
      - name: Second
        run: echo two
""",
    ["First", "Second"],
)

# A non-Windows job is not the guard's business at all.
verdict(
    "a macos-only job is left alone",
    HEAD
    + """  mac:
    runs-on: macos-latest
    steps:
      - name: Undeclared but unreachable on windows
        run: echo hi
""",
    [],
)

# An `if:` pinning the step to another OS makes it unreachable on
# Windows even inside a matrix that includes it.
verdict(
    "a step pinned to another OS by if: is unreachable",
    HEAD
    + """  matrixed:
    strategy:
      matrix:
        os: [ubuntu-latest, windows-latest]
    runs-on: ${{ matrix.os }}
    steps:
      - name: Linux only
        if: matrix.os == 'ubuntu-latest'
        run: echo hi
""",
    [],
)

# `uses:` steps have no shell to declare.
verdict(
    "a uses: step is not a run: step",
    HEAD
    + """  win:
    runs-on: windows-latest
    steps:
      - uses: actions/checkout@v4
      - name: Declared
        shell: bash
        run: echo hi
""",
    [],
)

# A job boundary must reset the platform, or one Windows job would make
# every job after it look like one.
verdict(
    "a windows job does not leak into the next job",
    HEAD
    + """  win:
    runs-on: windows-latest
    steps:
      - name: Declared
        shell: bash
        run: echo hi
  mac:
    runs-on: macos-latest
    steps:
      - name: Undeclared
        run: echo hi
""",
    [],
)


if failures:
    print("check-workflow-shells.py self-test FAILED:")
    for f in failures:
        print(f"  {f}")
    sys.exit(1)

print("check-workflow-shells.py self-test: the shell guard rejects what it should.")
