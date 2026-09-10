#!/usr/bin/env python3
"""The mobile jobs' path gate must be wired so it can only fail LOUDLY.

`mobile-android` and `mobile-ios` are required checks that skip their own
steps when a pull request touches no mobile code. A required check that
does not RUN blocks a merge forever, so the jobs always run and report
under their exact required names; what is conditional is their steps.

That shape has one catastrophic failure mode, and it is silent. Every
step is gated on `needs.mobile-changes.outputs.run == 'true'`. If a job
ever loses its `needs: mobile-changes`, that expression does not error --
it evaluates to the empty string, which is not `'true'`, so EVERY step is
skipped on EVERY run and the job reports green having compiled nothing.
Mobile would then be unverified indefinitely with a fully green board,
which is precisely the outcome the path filter was written not to cause.

The mirror-image failure is a step added later without the gate: that one
is merely wasteful (the job does work it was meant to skip) but it also
means the job is no longer honestly described by this comment, so it is
checked too.

Deliberately parses by indentation rather than importing PyYAML, the same
tradeoff `check-workflow-shells.py` makes: the lint runner has no
third-party Python packages, and a guard that needs its own install is a
guard that gets dropped.
"""

import pathlib
import re
import sys

WORKFLOW = pathlib.Path(".github/workflows/ci.yml")
GATE_JOB = "mobile-changes"
GATED_JOBS = ("mobile-android", "mobile-ios")
CONDITION = f"needs.{GATE_JOB}.outputs.run == 'true'"


def jobs_of(text: str) -> dict[str, list[str]]:
    """Split the file into {job name: its lines}. Jobs are at 2 spaces."""
    jobs: dict[str, list[str]] = {}
    current: str | None = None
    for line in text.splitlines():
        m = re.match(r"^  ([A-Za-z0-9_-]+):\s*$", line)
        if m:
            current = m.group(1)
            jobs[current] = []
            continue
        if current is not None:
            jobs[current].append(line)
    return jobs


def steps_of(lines: list[str]):
    """Yield the lines of each step. Steps start with `- ` at 6 spaces."""
    current: list[str] = []
    for line in lines:
        if re.match(r"^      - ", line):
            if current:
                yield current
            current = [line]
        elif current:
            # A line dedented past the step body ends it.
            if line.strip() and not line.startswith("        "):
                yield current
                current = []
            else:
                current.append(line)
    if current:
        yield current


def main() -> int:
    text = WORKFLOW.read_text()
    jobs = jobs_of(text)
    problems: list[str] = []

    if GATE_JOB not in jobs:
        print(f"{WORKFLOW}: the `{GATE_JOB}` job is gone; the mobile jobs")
        print("have nothing to read their condition from and would skip")
        print("every step while reporting success.")
        return 1

    for job in GATED_JOBS:
        if job not in jobs:
            problems.append(f"`{job}` is missing from {WORKFLOW}")
            continue

        body = jobs[job]

        # The load-bearing one. Without `needs`, the condition on every
        # step below silently evaluates to the empty string.
        needs = [l for l in body if re.match(r"^    needs:", l)]
        if not any(GATE_JOB in l for l in needs):
            problems.append(
                f"`{job}` has no `needs: {GATE_JOB}`, so every step's "
                f"`{CONDITION}` is false and the job would report green "
                f"without running anything"
            )

        for step in steps_of(body):
            step_body = "\n".join(step)
            if CONDITION in step_body:
                continue
            name = re.search(r"(?:name|uses):\s*(.+)$", step_body, re.M)
            label = name.group(1).strip() if name else "(unnamed)"
            problems.append(f"`{job}` step {label!r} is not gated on {CONDITION}")

    if problems:
        print("The mobile path gate is mis-wired:")
        for p in problems:
            print(f"  {p}")
        print()
        print("Every step of the mobile jobs must carry")
        print(f"  if: {CONDITION}")
        print(f"and each job must declare `needs: {GATE_JOB}`. See the")
        print(f"`{GATE_JOB}` job in {WORKFLOW} for why.")
        return 1

    print(f"The mobile path gate is wired correctly on {', '.join(GATED_JOBS)}.")
    return 0


if __name__ == "__main__":
    sys.exit(main())
