#!/usr/bin/env python3
"""Proof that the injected Gradle block is what we think it is.

The signing block this script writes could not be compiled by anyone on
this project -- no machine here has an Android SDK, and CI only
`cargo check`ed the Rust -- so its first Gradle compile was its first
test, and it failed (#564: `java.util.Properties` does not resolve in a
Kotlin DSL script).

These tests cannot compile Kotlin either. What they CAN do is pin the
two things that were actually wrong:

- the block must not reference a `java.*` type, since the Kotlin DSL's
  implicit imports do not cover them and this script cannot add an
  `import` without editing the top of a generated file, and
- the parsing it does must produce the four values a keystore file
  holds, which is checked by running the same algorithm here.

Everything past that -- does Gradle accept the syntax -- is the dry run's
job, and the PR that changes this file must show one.

Run: python3 scripts/android-release-signing.test.py
"""

import contextlib
import importlib.util
import io
import pathlib
import re
import sys
import tempfile

HERE = pathlib.Path(__file__).resolve().parent
spec = importlib.util.spec_from_file_location(
    "signing", HERE / "android-release-signing.py"
)
signing = importlib.util.module_from_spec(spec)
spec.loader.exec_module(signing)

failures: list[str] = []


def check(name: str, got, want):
    if got != want:
        failures.append(f"{name}\n    got:  {got!r}\n    want: {want!r}")


def ok(name: str, cond: bool, detail: str = ""):
    if not cond:
        failures.append(f"{name}{': ' + detail if detail else ''}")


# A generated file in the shape `tauri android init` writes: a plugins
# block first, then `android { }` holding `buildTypes` with a release
# type. The anchors this script replaces are all here.
GENERATED = """plugins {
    id("com.android.application")
    id("org.jetbrains.kotlin.android")
    id("rust")
}

android {
    compileSdk = 34
    namespace = "com.pktstorm.headstate.companion"
    defaultConfig {
        minSdk = 24
    }
    buildTypes {
        getByName("debug") {
            isDebuggable = true
        }
        getByName("release") {
            isMinifyEnabled = true
        }
    }
}
"""

patched = signing.patch(GENERATED)

# The bug itself: a `java.*` reference anywhere in what we inject is the
# failure mode, whatever the surrounding syntax looks like.
ok(
    "no java.* type in the injected block",
    not re.search(r"\bjava\.[a-z]", patched),
    "Kotlin DSL scripts do not import java.*; see #564",
)
ok(
    "no bare Properties() either",
    "Properties()" not in patched,
    "would need an import this script cannot place",
)

# The three anchors were each hit exactly once.
check("one marker", patched.count(signing.MARKER), 1)
ok("signingConfigs added", "signingConfigs {" in patched)
ok(
    "release build type keeps its own body",
    "isMinifyEnabled = true" in patched,
    "the hook must PREPEND to the block, not replace it",
)
ok(
    "signing is applied to the release type",
    'signingConfig = signingConfigs.getByName("release")' in patched,
)
ok(
    "debug build type untouched",
    patched.count('getByName("debug")') == 1 and "isDebuggable = true" in patched,
)

# Every read of the map is guarded on the file existing, so an unsigned
# build -- every dry run, every local build -- still configures.
for guarded in ("signingConfigs {", 'signingConfig = signingConfigs'):
    idx = patched.index(guarded)
    ok(
        f"{guarded.strip()} is guarded on the file existing",
        "keystorePropertiesFile.exists()" in patched[max(0, idx - 200) : idx + 80],
    )

# Idempotent: running twice must leave one copy.
ok("second run is a no-op", signing.MARKER in patched)
with tempfile.TemporaryDirectory() as d:
    p = pathlib.Path(d) / "build.gradle.kts"
    p.write_text(GENERATED)
    sys.argv = ["x", str(p)]
    # `main` reports what it did on stdout, which is right when a person
    # runs it and noise inside a lint step; only a FAILURE should print.
    quiet = io.StringIO()
    with contextlib.redirect_stdout(quiet):
        signing.main()
        once = p.read_text()
        signing.main()
    check("running twice changes nothing", p.read_text(), once)
    check("still one marker after two runs", once.count(signing.MARKER), 1)
    ok(
        "the second run says so rather than patching again",
        "already configured" in quiet.getvalue(),
    )


def parse_like_kotlin(text: str) -> dict[str, str]:
    """The injected block's algorithm, line for line.

    Kept in step with `PROPERTIES` by the assertions below, which fail if
    the Kotlin stops doing what this does.
    """
    out = {}
    for raw in text.splitlines():
        it = raw.strip()
        if it and not it.startswith("#") and not it.startswith("!") and "=" in it:
            out[it.split("=", 1)[0].strip()] = it.split("=", 1)[1].strip()
    return out


# What the release workflow actually writes, plus the comment shapes a
# hand-written file from Tauri's guide would carry.
KEYSTORE = """# written by the release workflow
storeFile=/home/runner/work/headstate/upload.keystore
storePassword=a pass with spaces
keyAlias=upload
keyPassword=a pass with spaces

! a bang comment
"""
parsed = parse_like_kotlin(KEYSTORE)
check("storeFile", parsed.get("storeFile"), "/home/runner/work/headstate/upload.keystore")
check("keyAlias", parsed.get("keyAlias"), "upload")
check(
    "a password containing spaces survives",
    parsed.get("storePassword"),
    "a pass with spaces",
)
check("comments and blanks are skipped", len(parsed), 4)

# The Kotlin must still be doing each of those things.
for fragment in (
    'it.startswith("#")'.replace("startswith", "startsWith"),
    'it.startsWith("!")',
    'it.contains("=")',
    'it.substringBefore("=").trim()',
    'it.substringAfter("=").trim()',
):
    ok(f"Kotlin still does {fragment}", fragment in signing.PROPERTIES)

# The guide's single-`password` fallback, which a hand-written file uses.
for key in ("storePassword", "keyPassword"):
    ok(
        f"{key} falls back to `password`",
        f'keystoreProperties["{key}"]\n                    ?: keystoreProperties["password"]'
        in signing.SIGNING_CONFIGS,
    )

if failures:
    print(f"android signing self-test: {len(failures)} failed\n", file=sys.stderr)
    for f in failures:
        print(f"  {f}", file=sys.stderr)
    sys.exit(1)
print("android signing self-test: clean")
