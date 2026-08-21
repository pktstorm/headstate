#!/usr/bin/env python3
"""Every `run:` step reachable on Windows must declare `shell: bash`.

Windows runners default to PowerShell, where `set -euo pipefail`, heredocs,
and `${VAR#prefix}` are all parse errors. The failure is late and confusing:
the step dies with a ParserError pointing at a generated .ps1 file, and only
on one platform.

Caught for real by the v2.0.1-rc.1 rehearsal, where "Stamp version from tag"
failed on Windows while macOS and Linux passed.
"""

import sys
import pathlib
import yaml

# A step is unreachable on Windows if its `if:` pins it to another OS.
NON_WINDOWS = ("macos-latest", "ubuntu-latest")


def reaches_windows(job: dict, step: dict) -> bool:
    runs_on = str(job.get("runs-on", ""))
    strategy = job.get("strategy", {}).get("matrix", {})
    include = strategy.get("include", [])
    oses = [str(e.get("os", "")) for e in include] or [runs_on]
    if not any("windows" in o for o in oses):
        return False
    cond = str(step.get("if", ""))
    return not any(o in cond for o in NON_WINDOWS)


def main() -> int:
    bad = []
    for path in sorted(pathlib.Path(".github/workflows").glob("*.yml")):
        doc = yaml.safe_load(path.read_text())
        for job_name, job in (doc.get("jobs") or {}).items():
            for step in job.get("steps") or []:
                if "run" not in step or "shell" in step:
                    continue
                if reaches_windows(job, step):
                    name = step.get("name", "(unnamed)")
                    bad.append(f"{path}: {job_name} / {name}")

    if bad:
        print("These `run:` steps can execute on Windows without `shell: bash`:")
        for b in bad:
            print(f"  {b}")
        print("\nWindows defaults to PowerShell, where bash syntax is a parse error.")
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())
