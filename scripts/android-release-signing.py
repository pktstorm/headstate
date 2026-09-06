#!/usr/bin/env python3
"""Wire release signing into the generated Android project, once.

`tauri android init` writes `src-mobile/gen/android/app/build.gradle.kts`
with no signing configuration, so `tauri android build --aab` produces an
UNSIGNED bundle -- which Google Play refuses. Tauri's Android signing guide
(https://v2.tauri.app/distribute/sign/android/) says to add a
`signingConfigs` block that reads `gen/android/keystore.properties`; this
script adds that block so the release workflow does not depend on a hand
edit that `tauri android init` would silently drop on regeneration.

Three deliberate departures from the guide's snippet:

1. Everything is guarded on `keystore.properties` existing. The guide's
   version does `keystoreProperties["keyAlias"] as String`, which throws
   when the file is absent -- and it IS absent on every dry run and every
   local build. An unsigned build must still build.
2. `storePassword` and `keyPassword` are read separately, falling back to
   the guide's single `password` key, so a file written by hand from the
   guide still works. (A PKCS12 keystore, the `keytool` default, requires
   the two to be equal anyway.)
3. The file is parsed into a `Map<String, String>` rather than with
   `java.util.Properties`, which the guide uses. A Gradle Kotlin DSL
   script is compiled against a FIXED implicit-import set that does not
   include `java.util`, so even the fully-qualified `java.util.Properties()`
   does not resolve -- `Unresolved reference: util`, and the build dies
   before it compiles a line of the app (#564). An `import` would work,
   but it would have to go at the top of a GENERATED file this script
   patches by string replacement, above blocks whose shape is Tauri's to
   change; a self-contained expression has no such ordering constraint.
   The parser handles what this file actually holds -- four unquoted
   values written by the release workflow, or by hand from the guide --
   and deliberately not the whole `.properties` grammar (escapes,
   continuations, `:` as a separator), which nothing here emits.

Idempotent: a marker comment records that the block is present, so
running this on an already-patched file is a no-op. That is what lets the
workflow call it unconditionally, whether `gen/android` was just
generated or has since been committed with the patch applied.

Usage: android-release-signing.py [path/to/app/build.gradle.kts]
"""

import pathlib
import sys

DEFAULT_PATH = "src-mobile/gen/android/app/build.gradle.kts"
MARKER = "// headstate: release signing (scripts/android-release-signing.py)"

PROPERTIES = f"""{MARKER}
val keystorePropertiesFile = rootProject.file("keystore.properties")
val keystoreProperties: Map<String, String> =
    if (keystorePropertiesFile.exists()) {{
        keystorePropertiesFile.readLines()
            .map {{ it.trim() }}
            .filter {{ it.isNotEmpty() && !it.startsWith("#") && !it.startsWith("!") && it.contains("=") }}
            .associate {{ it.substringBefore("=").trim() to it.substringAfter("=").trim() }}
    }} else {{
        emptyMap()
    }}

"""

SIGNING_CONFIGS = """    signingConfigs {
        if (keystorePropertiesFile.exists()) {
            create("release") {
                storeFile = file(keystoreProperties["storeFile"]!!)
                storePassword = keystoreProperties["storePassword"]
                    ?: keystoreProperties["password"]
                keyAlias = keystoreProperties["keyAlias"]
                keyPassword = keystoreProperties["keyPassword"]
                    ?: keystoreProperties["password"]
            }
        }
    }
"""

RELEASE_HOOK = """        getByName("release") {
            if (keystorePropertiesFile.exists()) {
                signingConfig = signingConfigs.getByName("release")
            }
"""


def replace_once(text: str, old: str, new: str, what: str) -> str:
    count = text.count(old)
    if count != 1:
        sys.exit(f"expected exactly one {what} in build.gradle.kts, found {count}")
    return text.replace(old, new, 1)


def patch(text: str) -> str:
    text = replace_once(text, "\nandroid {\n", "\n" + PROPERTIES + "android {\n", "`android {` block")
    text = replace_once(text, "    buildTypes {\n", SIGNING_CONFIGS + "    buildTypes {\n", "`buildTypes {` block")
    text = replace_once(text, '        getByName("release") {\n', RELEASE_HOOK, "release build type")
    return text


def main() -> int:
    path = pathlib.Path(sys.argv[1] if len(sys.argv) > 1 else DEFAULT_PATH)
    if not path.is_file():
        sys.exit(f"{path} does not exist; run `make android-init` first")
    text = path.read_text()
    if MARKER in text:
        print(f"{path}: release signing already configured")
        return 0
    path.write_text(patch(text))
    print(f"{path}: release signing configured (reads gen/android/keystore.properties when present)")
    return 0


if __name__ == "__main__":
    sys.exit(main())
