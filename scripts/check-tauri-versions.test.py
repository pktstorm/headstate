#!/usr/bin/env python3
"""Proof that the Tauri version guard can fail.

A guard nobody has watched fail is a guard that might be checking
nothing. These drive the parsers and the comparison directly, against
the real shapes from this repo's lockfiles, including the exact mismatch
from #552 that motivated the script (#555).

Run: python3 scripts/check-tauri-versions.test.py
"""

import importlib.util
import pathlib
import sys

HERE = pathlib.Path(__file__).resolve().parent
spec = importlib.util.spec_from_file_location("guard", HERE / "check-tauri-versions.py")
guard = importlib.util.module_from_spec(spec)
spec.loader.exec_module(guard)

failures: list[str] = []


def check(name: str, got, want):
    if got != want:
        failures.append(f"{name}\n    got:  {got!r}\n    want: {want!r}")


# The pairing rule, derived rather than listed so a new plugin is
# covered without editing the script.
check("api pairs with the tauri crate", guard.crate_for("@tauri-apps/api"), "tauri")
check(
    "a plugin pairs with tauri-plugin-*",
    guard.crate_for("@tauri-apps/plugin-updater"),
    "tauri-plugin-updater",
)
check(
    "an unknown shape pairs with nothing",
    guard.crate_for("@tauri-apps/something-else"),
    "",
)

# yarn.lock, in the shape Yarn 4 writes.
YARN = '''"@tauri-apps/plugin-updater@npm:^2.10.1":
  version: 2.11.0
  resolution: "@tauri-apps/plugin-updater@npm:2.11.0"
  dependencies:
    "@tauri-apps/api": "npm:^2.9.0"

"@tauri-apps/api@npm:^2, @tauri-apps/api@npm:^2.9.0":
  version: 2.9.0
  resolution: "@tauri-apps/api@npm:2.9.0"

"lucide-react@npm:1.41.0":
  version: 1.41.0
'''
resolved = guard.yarn_resolved(YARN)
check(
    "reads the resolved npm version",
    resolved.get("@tauri-apps/plugin-updater"),
    "2.11.0",
)
check(
    "reads a package sharing one entry with another descriptor",
    resolved.get("@tauri-apps/api"),
    "2.9.0",
)
check("ignores unrelated packages", "lucide-react" in resolved, False)

# Cargo.lock, in the shape cargo writes.
CARGO = '''[[package]]
name = "tauri-plugin-updater"
version = "2.10.1"
source = "registry+https://github.com/rust-lang/crates.io-index"

[[package]]
name = "tauri"
version = "2.11.5"
source = "registry+https://github.com/rust-lang/crates.io-index"
'''
crates = guard.cargo_resolved(CARGO)
check("reads the resolved crate version", crates.get("tauri-plugin-updater"), "2.10.1")
check("reads more than one crate", crates.get("tauri"), "2.11.5")

# The comparison itself. Tauri's rule is major/minor, so a patch
# difference is fine and a minor difference is not -- which is exactly
# the #552 case: 2.10.1 against 2.11.0.
check("the #552 mismatch is caught", guard.major_minor("2.10.1") != guard.major_minor("2.11.0"), True)
check("a patch difference is allowed", guard.major_minor("2.11.5") == guard.major_minor("2.11.0"), True)
check("a major difference is caught", guard.major_minor("1.0.0") != guard.major_minor("2.0.0"), True)
check("a version with no minor reads as .0", guard.major_minor("2"), ("2", "0"))

if failures:
    print(f"tauri guard self-test: {len(failures)} failed\n", file=sys.stderr)
    for f in failures:
        print(f"  {f}", file=sys.stderr)
    sys.exit(1)
print("tauri guard self-test: clean")
