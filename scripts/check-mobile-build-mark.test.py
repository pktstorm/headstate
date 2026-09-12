#!/usr/bin/env python3
"""Self-test for check-mobile-build-mark.py.

This guard's failure mode is a silent pass -- it is a network-dependent
check whose "cannot look" path exits 0 by default, so a bug that made it
ALWAYS take that path would leave the mark unguarded while printing
something reassuring. The cases below pin the decision table, especially
the absent-is-not-zero one: no evidence must never be reported as
"highest shipped is 0".

Uses the real module with `shipped_builds` monkeypatched, so the parsing,
the comparison and the exit codes under test are the ones that ship. The
network is never touched.
"""

import importlib.util
import io
import pathlib
import sys
import tempfile
import unittest.mock

HERE = pathlib.Path(__file__).resolve().parent
spec = importlib.util.spec_from_file_location(
    "mark_guard", HERE / "check-mobile-build-mark.py"
)
guard = importlib.util.module_from_spec(spec)
spec.loader.exec_module(guard)

FAILURES = []


def check(name: str, cond: bool, detail: str = "") -> None:
    if cond:
        print(f"  ok   {name}")
    else:
        print(f"  FAIL {name} {detail}")
        FAILURES.append(name)


def run(mark_text: str, builds, why, require: bool):
    """Run main() with a temp mark file and a stubbed asset lookup."""
    with tempfile.TemporaryDirectory() as d:
        path = pathlib.Path(d) / "mark"
        path.write_text(mark_text)
        argv = ["prog"] + (["--require"] if require else [])
        out = io.StringIO()
        with unittest.mock.patch.object(guard, "MARK_FILE", path), \
             unittest.mock.patch.object(
                 guard, "shipped_builds", lambda: (builds, why)), \
             unittest.mock.patch.object(sys, "stdout", out):
            code = guard.main(argv)
        return code, out.getvalue()


print("check-mobile-build-mark self-test")

# --- The comparison ---------------------------------------------------
code, out = run("# c\n28\n", {"mobile-v0.10.0": 29}, None, True)
check("a mark below the newest asset fails", code == 1)
check("...and names the value to write", "to 29" in out, out)

code, _ = run("# c\n29\n", {"mobile-v0.10.0": 29}, None, True)
check("a mark equal to the newest asset passes", code == 0)

# Ahead is legitimate: the file is written before the upload, so a run
# whose publish failed raises the mark with no asset behind it.
code, out = run("# c\n30\n", {"mobile-v0.10.0": 29}, None, True)
check("a mark ahead of the newest asset passes", code == 0)
check("...and says why that is fine", "before the upload" in out, out)

# Highest wins, not newest-listed: a re-tag can publish out of order.
code, _ = run(
    "# c\n29\n", {"mobile-v0.9.0": 28, "mobile-v0.10.0": 29}, None, True
)
check("the HIGHEST build across releases is used", code == 0)

# --- Absent is not zero ----------------------------------------------
code, out = run("# c\n28\n", {}, "the API is unreachable", True)
check("no evidence under --require fails", code == 1)
check("...and says it cannot tell", "Cannot determine" in out, out)
check("...and does NOT claim a highest of 0", "is 0" not in out, out)
check("...and says the mark went unchecked", "NOT checked" in out, out)

code, out = run("# c\n28\n", {}, "the API is unreachable", False)
check("no evidence without --require is advisory", code == 0)
check("...and still refuses to invent a number", "Cannot determine" in out, out)

# A low mark plus no evidence must not be blessed as a pass.
code, _ = run("# c\n1\n", {}, "the API is unreachable", True)
check("a badly stale mark is not blessed by a lookup failure", code == 1)

# --- Parsing ----------------------------------------------------------
check(
    "a build asset name parses",
    guard.ASSET_BUILD.search(
        "Headstate-Companion-0.10.0-build29.ipa"
    ).group(1) == "29",
)
check(
    "an aab parses too",
    guard.ASSET_BUILD.search(
        "Headstate-Companion-0.10.0-build29.aab"
    ).group(1) == "29",
)
check(
    "SHA256SUMS does not parse",
    guard.ASSET_BUILD.search("SHA256SUMS") is None,
)
# A version string containing the word must not be mistaken for the build.
check(
    "only the trailing build<N> matches",
    guard.ASSET_BUILD.search(
        "Headstate-Companion-build7-1.2.3-build29.ipa"
    ).group(1) == "29",
)

# The mark parser must agree with mobile-release.yml's shell one.
with tempfile.TemporaryDirectory() as d:
    p = pathlib.Path(d) / "m"
    p.write_text("# comment 99\n#  another\n28\n")
    with unittest.mock.patch.object(guard, "MARK_FILE", p):
        check("comments are stripped, not parsed", guard.read_mark() == 28)
    p.write_text("# c\n  28  \n")
    with unittest.mock.patch.object(guard, "MARK_FILE", p):
        check("surrounding whitespace is stripped", guard.read_mark() == 28)

print()
if FAILURES:
    print(f"{len(FAILURES)} failure(s): {', '.join(FAILURES)}")
    sys.exit(1)
print("all cases pass")
