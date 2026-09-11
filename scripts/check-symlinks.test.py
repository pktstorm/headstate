#!/usr/bin/env python3
"""Proof that the symlink guard can fail, and on what.

A guard nobody has watched fail is a guard that might be checking
nothing -- and this one's whole value is the NEXT escaping link rather
than the one that prompted it (#813), so the containment rule is the part
that has to be pinned.

Drives `escapes()` directly, which is where the judgement lives.
`tracked_symlinks()` is deliberately not exercised: it is three git
subprocesses with no decisions in them, and faking an index to test it
would test the fake.

The first case is the real blob from #813, verbatim in shape.

Run: python3 scripts/check-symlinks.test.py
"""

import importlib.util
import pathlib
import sys

HERE = pathlib.Path(__file__).resolve().parent
spec = importlib.util.spec_from_file_location("guard", HERE / "check-symlinks.py")
guard = importlib.util.module_from_spec(spec)
spec.loader.exec_module(guard)

failures: list[str] = []


def rejected(name: str, path: str, target: str):
    if guard.escapes(path, target) is None:
        failures.append(f"{name}\n    {path} -> {target} was allowed, should be rejected")


def allowed(name: str, path: str, target: str):
    why = guard.escapes(path, target)
    if why is not None:
        failures.append(f"{name}\n    {path} -> {target} was rejected as {why!r}")


# The #813 instance: an absolute path into one developer's home
# directory, which could never have resolved anywhere else.
#
# Spelled with the `acme` placeholder this repo uses for checkout paths,
# not the real one. `scripts/check-privacy.sh` rejects a
# `/Users/<name>/code/<owner>/<repo>`-shaped literal in a tracked file,
# and it is right to -- it caught this line on the first run.
rejected(
    "the absolute node_modules link from #813",
    "node_modules",
    "/Users/dev/code/acme/widget/node_modules",
)
rejected("any absolute target", "config", "/etc/hosts")
rejected(
    "a sibling checkout reached with ..",
    "src/shared",
    "../../other-repo/shared",
)
rejected(
    "climbing out from a nested path",
    "src-tauri/vendor/thing",
    "../../../thing",
)
# `..` that merely returns to the root is still inside: `a/b` -> `../c`
# is `c`. The guard must not reject on seeing `..` at all, or every
# legitimate relative link across directories fails.
allowed("'..' that stays inside is fine", "a/b", "../c")
allowed("a sibling file", "src-tauri/icons/icon.png", "base.png")
allowed("a nested relative target", "docs/readme", "../README.md")
allowed("an explicit './' prefix", "docs/readme", "./README.md")
# The pathological case: exactly as many `..` as there are segments
# lands ON the root, which is inside. One more escapes.
allowed("landing exactly on the root", "a/b/c", "../../d")
rejected("one segment past the root", "a/b/c", "../../../d")

if failures:
    print("symlink guard tests FAILED:")
    for f in failures:
        print(f"  {f}")
    sys.exit(1)

print("symlink guard tests: all pass")
