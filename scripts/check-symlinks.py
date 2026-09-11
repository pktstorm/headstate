#!/usr/bin/env python3
"""No committed symlink may point outside the repository.

A symlink in a git tree is a mode 120000 blob whose CONTENT is the target
path. Git records it verbatim and checks it out verbatim, so a target
that is absolute, or that climbs out of the repo with `..`, names
something that exists only on the machine where it was made. It can
never resolve correctly anywhere else -- not on a colleague's checkout,
not on a CI runner, not in a fresh worktree of the same repo.

Why a guard and not just the ignore rule (#813). `.gitignore:1` was
`node_modules/`, and the trailing slash matches a DIRECTORY only, so a
SYMLINK named `node_modules` -> `/Users/<someone>/code/<repo>/node_modules`
was committed without a warning. Dropping the slash fixes that name. It
does not fix the NEXT one: the same `ln -s` reflex in a fresh worktree
reaches for `target`, `dist`, a fixture directory, a shared `.env` --
and an ignore rule can only ever enumerate names somebody already
thought of. This asks the general question instead, which is the one
that actually holds: does this link point somewhere the repository
controls?

It is also a supply-chain check, not only a portability one, which is
the stronger half of the argument for keeping it. A tracked symlink
escaping the tree is how a build step gets aimed at content the
repository does not contain and review never sees -- `config` ->
`/etc/something`, or a source directory redirected through `../..` into a
sibling checkout. Nothing in this project legitimately does that, so the
rule can be absolute rather than an allow-list, and an absolute rule is
one nobody has to maintain.

What it cost to find the instance without this: nine of ten CI jobs
failing deterministically with `ENOTDIR: not a directory, mkdir
'.../node_modules'`, the package named in the error varying per run so it
read as a flaky registry fetch, three CI runs, and one wrong fix shipped
(#811, an install retry for a transient that was not the cause). The
authoring machine could not reproduce it, because there a real
`node_modules` directory existed for the link to resolve to.

Relative links that STAY inside the repo are allowed deliberately, not
overlooked. `crates/*/` path dependencies and an icon pointing at a
sibling asset are ordinary, portable, and resolve identically everywhere
-- the property that matters is containment, not relativity.

Reads `git ls-files -s` rather than walking the filesystem, which is the
point: the question is what is COMMITTED, and a working tree can hold a
local symlink that is correctly ignored (the node_modules one was fine on
disk; it was only committing it that broke anything). No third-party
packages, like the other guards here -- a guard that needs its own
install is a guard that gets dropped.
"""

import pathlib
import subprocess
import sys

# Git's mode for a symbolic link. Mode, not a filename heuristic: it is
# the only thing that actually distinguishes a link blob from a text file
# whose contents happen to look like a path.
SYMLINK_MODE = "120000"


def tracked_symlinks() -> list[tuple[str, str]]:
    """Every committed symlink, as (path, target)."""
    index = subprocess.run(
        ["git", "ls-files", "-s", "-z"],
        capture_output=True,
        text=True,
        check=True,
    ).stdout

    links = []
    for entry in index.split("\0"):
        if not entry:
            continue
        # `<mode> <sha> <stage>\t<path>`
        meta, _, path = entry.partition("\t")
        if not meta.startswith(SYMLINK_MODE + " "):
            continue
        sha = meta.split()[1]
        # The blob content IS the target path. Read it from the object
        # store rather than from disk: on a platform or filesystem
        # without symlink support git checks the link out as a regular
        # file containing the path, and then `readlink` would fail on
        # exactly the machines most likely to be broken by the link.
        target = subprocess.run(
            ["git", "cat-file", "blob", sha],
            capture_output=True,
            text=True,
            check=True,
        ).stdout
        links.append((path, target.strip()))
    return links


def escapes(path: str, target: str) -> str | None:
    """Why this link leaves the repository, or None if it stays inside."""
    if target.startswith("/"):
        return "absolute target"
    # Resolved LEXICALLY against the link's own directory, with no
    # filesystem access. `Path.resolve()` would consult the real disk,
    # where an intermediate component may be missing (CI checkouts are
    # sparse of build output) or may itself be a link -- and either would
    # make the verdict depend on the machine running the check, which is
    # the class of bug this guard exists to catch.
    parts = list(pathlib.PurePosixPath(path).parent.parts)
    for part in pathlib.PurePosixPath(target).parts:
        if part == ".":
            continue
        if part == "..":
            if not parts:
                return "climbs above the repository root with '..'"
            parts.pop()
        else:
            parts.append(part)
    return None


def main() -> int:
    bad = [
        (path, target, why)
        for path, target in tracked_symlinks()
        if (why := escapes(path, target))
    ]

    if bad:
        print("These committed symlinks point outside the repository:")
        for path, target, why in bad:
            print(f"  {path} -> {target}  ({why})")
        print()
        print("Such a link resolves only on the machine that made it, so it")
        print("breaks every other checkout and every CI runner -- and a link")
        print("escaping the tree can aim a build at content review never sees.")
        print("Remove it (`git rm --cached <path>`) and ignore the name")
        print("instead; see scripts/check-symlinks.py and #813.")
        return 1

    print("symlink check: no committed symlink escapes the repository.")
    return 0


if __name__ == "__main__":
    sys.exit(main())
