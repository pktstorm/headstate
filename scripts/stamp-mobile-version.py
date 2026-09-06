#!/usr/bin/env python3
"""Stamp the mobile companion's version from a `mobile-v*` tag.

The desktop's release.yml does this inline for src-tauri; the mobile
release workflow has two build jobs (iOS and Android) that must agree on
the version, so the same logic lives here once rather than twice.

Writes the version into `src-mobile/tauri.conf.json` (which Tauri turns
into CFBundleShortVersionString and versionName at build time) and
`src-mobile/Cargo.toml`. The edits are for one build and are never
committed: the tag already records the version, and stamping rather than
committing a bump keeps the two fields from drifting apart.

Targeted substitution rather than a JSON round-trip: json.dump reflows
inline arrays, which would bury the one real change in cosmetic noise in
the CI log. Each pattern is anchored to the top-level version key and
count=1 stops it reaching anything further down.

Usage:
  stamp-mobile-version.py X.Y.Z   stamp both files
  stamp-mobile-version.py --show  print the version the tree declares
"""

import re
import sys

TAURI_CONF = "src-mobile/tauri.conf.json"
CARGO_TOML = "src-mobile/Cargo.toml"


def show() -> int:
    text = open(TAURI_CONF).read()
    m = re.search(r'"version"\s*:\s*"([^"]+)"', text)
    if not m:
        sys.exit(f"no version key found in {TAURI_CONF}")
    print(m.group(1))
    return 0


def stamp(version: str) -> int:
    text = open(TAURI_CONF).read()
    text, n = re.subn(r'("version"\s*:\s*)"[^"]+"', rf'\g<1>"{version}"', text, count=1)
    assert n == 1, f"no version key found in {TAURI_CONF}"
    open(TAURI_CONF, "w").write(text)

    cargo = open(CARGO_TOML).read()
    cargo, n = re.subn(r'^version = "[^"]+"', f'version = "{version}"', cargo, count=1, flags=re.M)
    assert n == 1, f"no [package] version found in {CARGO_TOML}"
    open(CARGO_TOML, "w").write(cargo)
    print(f"stamped {version} into {TAURI_CONF} and {CARGO_TOML}")
    return 0


def main() -> int:
    if len(sys.argv) != 2:
        sys.exit(__doc__)
    if sys.argv[1] == "--show":
        return show()
    return stamp(sys.argv[1])


if __name__ == "__main__":
    sys.exit(main())
