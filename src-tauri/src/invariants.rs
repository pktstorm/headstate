//! Rules this codebase already states, asserted over its own source.
//!
//! # Why this module exists (#854)
//!
//! Six parallel audits of v5.13.0 produced about fifty findings, and the
//! thing every auditor noticed independently was that almost none was a
//! conceptual gap. The security auditor put it best: *"In every case the
//! correct implementation already exists elsewhere in this same codebase,
//! usually with a doc comment explaining the threat. None of these are
//! conceptual gaps in the authors' understanding; they are places a known
//! rule was not propagated to a sibling."*
//!
//! Fifty patches fix fifty instances and prevent none. The rule was
//! known every time -- written down, often measured -- and it still did
//! not reach the sibling, so the fifty-first arrives next release. What
//! closes that is a test that fails the moment the sibling is written.
//!
//! # Why the checks here are source scans
//!
//! Each property below is a statement about EVERY call site rather than
//! about observable behaviour at one of them. A behavioural test proves
//! the mechanism works -- which is rarely the thing in doubt -- while
//! saying nothing about the next call site somebody adds. That argument
//! is not new here: `stats::fetch`'s
//! `every_stats_read_goes_through_the_process_wide_permit` makes it at
//! length, and `stats::budget`'s `every_stats_query_meters_itself` is the
//! model this module follows throughout.
//!
//! # Derived, not enumerated
//!
//! The one hard lesson of #844, #842 and #847 is that a hand-written list
//! cannot cover the item nobody remembered to add to it -- all three
//! defects were exactly that, and #844's metering guard found two
//! uncovered documents the moment it stopped naming six. So every scan
//! here DISCOVERS its subjects:
//!
//! - the files, by walking the crate tree at runtime rather than by
//!   `include_str!` on a fixed list, so a brand-new file is covered
//!   without anyone remembering to add it. `include_str!` is the existing
//!   idiom and it is the one thing it cannot do: `every_query_document`
//!   names two paths, which is why `stats/tree.rs`' two documents sit
//!   outside it to this day.
//! - the call sites, by scanning that text.
//!
//! # What a source scan cannot see, stated rather than glossed
//!
//! These limits are real and shared by every check below:
//!
//! - **It cannot follow a call.** A check performed in a helper is
//!   invisible, so where a guard would otherwise report a defect at a
//!   location that does not have one, it offers a NAMED exemption with a
//!   recorded reason rather than a loose pattern.
//! - **It reads text, not semantics.** `"remove_dir_all"` inside a string
//!   literal or a doc comment looks like a call. Comment lines are
//!   skipped explicitly for that reason.
//! - **It sees only this crate tree.** A removal performed by a
//!   dependency, or through `std::process::Command`, is outside every
//!   check here.
//! - **It cannot prove a check is CORRECT**, only that one is present.
//!   `symlink_metadata` called and its answer ignored would pass. The
//!   behavioural tests beside each gate are what cover that, and they
//!   are cited from the checks that depend on them.
//!
//! A guard that cries wolf gets disabled -- `check-privacy.sh:120`
//! records ~40 false positives from one unanchored pattern as the reason
//! every pattern in it is anchored. So each check here prefers an
//! explicit, commented allowlist over a broader regex, and every entry in
//! one says why it is there.

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;
    use std::path::{Path, PathBuf};

    /// The crates this repository builds, and where their Rust lives.
    ///
    /// Walked at runtime from `CARGO_MANIFEST_DIR` (the idiom
    /// `packages::detect` and `tray` already use to reach repository
    /// files from a test) rather than listed as `include_str!` paths,
    /// which is the whole point: a new file under any of these is
    /// covered the moment it is written.
    ///
    /// `src-mobile` and `crates/headstate-stepup` are separate crates
    /// that `cargo test` here does not compile, and that is exactly why
    /// they are read as TEXT. The alternative -- a copy of each check in
    /// each crate -- is the duplication this module exists to stop.
    fn crate_roots() -> Vec<(&'static str, PathBuf)> {
        let manifest = Path::new(env!("CARGO_MANIFEST_DIR"));
        vec![
            ("src-tauri", manifest.join("src")),
            ("src-mobile", manifest.join("../src-mobile/src")),
            ("stepup", manifest.join("../crates/headstate-stepup/src")),
        ]
    }

    /// Every `.rs` file under `dir`, recursively.
    ///
    /// Sorted, so a failure message names the same file every run: an
    /// unstable order in a guard's output makes two identical failures
    /// look like two different ones.
    fn rust_files(dir: &Path) -> Vec<PathBuf> {
        let mut out = Vec::new();
        let mut stack = vec![dir.to_path_buf()];
        while let Some(d) = stack.pop() {
            let Ok(entries) = std::fs::read_dir(&d) else {
                continue;
            };
            for e in entries.flatten() {
                let p = e.path();
                if p.is_dir() {
                    stack.push(p);
                } else if p.extension().is_some_and(|x| x == "rs") {
                    out.push(p);
                }
            }
        }
        out.sort();
        out
    }

    /// The PRODUCTION code in a file: everything outside a
    /// `#[cfg(test)]` item.
    ///
    /// Every check here is about shipped code. Test code legitimately
    /// does the things these guards forbid -- a fixture deletes its own
    /// temp directory, a test names a command to assert it is refused --
    /// and a guard that failed on its own fixtures would be turned off
    /// within a day.
    ///
    /// # Why this is not a split on the first marker
    ///
    /// `stats::fetch`'s permit guard does exactly that
    /// (`src.split_once("\n#[cfg(test)]")`), and it is correct for the
    /// four files it reads, each of which ends in one test module. It is
    /// WRONG in general, and this guard caught it the first time it ran:
    /// `caches/mod.rs` has a test module at line 291 and more production
    /// code after it, including the `remove_dir_all` in `remove_venv` at
    /// 565. Splitting on the first marker hid one of the three call sites
    /// the invariant exists to check -- and the only thing that said so
    /// was the `checked >= 3` self-guard at the bottom of the test.
    /// Without that assertion this would have shipped green while seeing
    /// two thirds of the code.
    ///
    /// So each `#[cfg(test)]` item is skipped individually, and the end
    /// of one is found by INDENTATION rather than by counting braces.
    ///
    /// Brace-counting was written first and was wrong, which is worth
    /// recording because the failure is not obvious: `scan.rs`' test
    /// module contains `"@{u}"` and `"@{{u}}"` -- git's upstream syntax,
    /// in assertion messages -- and a counter that does not lex string
    /// literals reads those as real braces. It closed the 3,600-line
    /// module 1,400 lines early and then judged the test code in the
    /// remainder by production rules, reporting
    /// `reasons_are_display_ready_and_pluralised` as an unguarded
    /// recursive delete. A guard that cries wolf gets disabled, so the
    /// mechanism had to change rather than the message.
    ///
    /// Indentation is reliable here for a reason specific to this
    /// codebase rather than by luck: `cargo fmt --check` runs on all
    /// three crates in `make lint`, so every top-level item begins at
    /// column 0 and everything inside a module is indented. A
    /// `#[cfg(test)]` item therefore ends at the next line that starts
    /// with a non-space, non-`}` character.
    ///
    /// Column 0 alone is not enough, which was the second wrong version:
    /// `scan.rs`' `SAMPLE` fixture is a line-continuation string literal
    /// whose CONTENT starts at column 0 (`worktree /home/u/code/...`),
    /// and that ended the test module 5,000 lines early. So the
    /// terminator must also LOOK like a Rust item -- one of the keywords
    /// a top-level item can begin with, or an attribute. A string
    /// literal's contents do not, and neither does prose.
    ///
    /// The limits that survive, and they are real: a `#[cfg(test)]` on a
    /// nested (indented) item is not recognised at all, and a raw string
    /// whose content begins with one of those keywords at column 0 would
    /// still end a block early. Neither occurs in this tree, and both
    /// fail in the direction of seeing MORE code rather than less -- a
    /// false positive somebody reads, not a silent gap. The `checked`
    /// self-guard at the bottom of each test is what catches the other
    /// direction.
    fn production(src: &str) -> String {
        let mut out = String::new();
        // `Some(true)` while the skipped item's own header line
        // (`mod tests {`, `fn helper() {`) is still to be consumed: that
        // line is itself at column 0, so looking for the terminator
        // before eating it ends every block after one line -- which is
        // how this first reported a test in `auth.rs` as production code.
        let mut skipping: Option<bool> = None;
        for line in src.lines() {
            let top_level = !line.starts_with([' ', '\t']) && !line.is_empty();
            match skipping {
                Some(true) => {
                    skipping = Some(false);
                    continue;
                }
                Some(false) => {
                    // The first top-level line that begins a new Rust
                    // ITEM ends the block. "Looks like an item" rather
                    // than merely "is at column 0", because a string
                    // literal's content can sit at column 0 too.
                    if top_level && starts_an_item(line) {
                        skipping = None;
                    } else {
                        continue;
                    }
                }
                None => {}
            }
            if line.trim_start().starts_with("#[cfg(test)]") {
                skipping = Some(true);
                continue;
            }
            out.push_str(line);
            out.push('\n');
        }
        out
    }

    /// Whether a line begins a top-level Rust item.
    ///
    /// The vocabulary a `.rs` file's column 0 can legitimately start
    /// with, which is short and closed. Used to tell the end of a
    /// `#[cfg(test)]` block from a line of string-literal content that
    /// merely happens to be unindented -- see [`production`].
    fn starts_an_item(line: &str) -> bool {
        const ITEM: &[&str] = &[
            "fn ",
            "pub ",
            "mod ",
            "use ",
            "const ",
            "static ",
            "struct ",
            "enum ",
            "impl ",
            "trait ",
            "type ",
            "macro_rules!",
            "extern ",
            "unsafe ",
            "async ",
            "#[",
            "#!",
            "///",
            "//!",
            "//",
        ];
        ITEM.iter().any(|k| line.starts_with(k))
    }

    /// Whether a line is a comment, and so a mention rather than code.
    ///
    /// Load-bearing for every scan here: this codebase documents its
    /// rules at length directly above the code that implements them, so
    /// the name a guard greps for appears in prose far more often than in
    /// a call. Without this every check below would fire on the very doc
    /// comments that state the rule it enforces.
    fn is_comment(line: &str) -> bool {
        let t = line.trim_start();
        t.starts_with("//") || t.starts_with("*") || t.starts_with("#!")
    }

    /// The body of the function containing byte offset `at`, as text.
    ///
    /// Scoped between the nearest preceding `fn` and the next top-level
    /// item, so a check has to be in THIS function rather than merely
    /// somewhere in a file that has many. `every_stats_query_meters_itself`
    /// records what getting this wrong costs: an earlier version anchored
    /// on the first mention of a name instead of its definition, read the
    /// wrong region, and reported a defect at a location that did not have
    /// one.
    ///
    /// Returns the name as well, so a failure can say which function.
    fn enclosing_fn(src: &str, at: usize) -> (String, String) {
        let before = &src[..at];
        let start = ["\nfn ", "\npub fn ", "\n    fn ", "\n    pub fn "]
            .iter()
            .filter_map(|m| before.rfind(m))
            .max()
            .unwrap_or(0);
        let body = &src[start..];
        // To the next item, searched FROM the hit rather than from the
        // start of the body.
        //
        // `body.find(m)` finds each pattern's FIRST occurrence, which may
        // already be behind the hit -- and filtering those out discards
        // the pattern entirely instead of looking for its next occurrence,
        // so the body runs on to whichever pattern happens to appear
        // later. The same mistake in `client.rs`' refusal guard gave one
        // function a body spanning six others, which made that guard pass
        // over the defect it was written for. It was caught by reverting
        // the fix and watching the guard NOT fail.
        let rel = at - start;
        let end = ["\nfn ", "\npub fn ", "\n}\n"]
            .iter()
            .filter_map(|m| body[rel..].find(m).map(|e| rel + e))
            .min()
            .unwrap_or(body.len());
        let body = &body[..end];
        let name = body
            .split_once("fn ")
            .and_then(|(_, r)| r.split(['(', '<']).next())
            .unwrap_or("<unknown>")
            .trim()
            .to_string();
        (name, body.to_string())
    }

    // ---- Invariant 1: recursive deletion ---------------------------------

    /// `remove_dir_all` on an externally-supplied path sits behind a
    /// symlink check AND a containment check.
    ///
    /// # What it enforces
    ///
    /// Every production call to `std::fs::remove_dir_all` must be in a
    /// function that also calls `symlink_metadata` (and tests
    /// `is_symlink`) and that checks the canonical path is inside a root
    /// it was GIVEN, rather than one the caller named.
    ///
    /// # The finding it would have caught (#854, the #841 family)
    ///
    /// `worktrees::remove_orphan` had NEITHER. Its gate was
    /// `orphan_gitdir`, which asks for a readable `<dir>/.git` whose
    /// `gitdir:` target does not exist -- a two-line file any caller can
    /// write, and not a containment boundary in any case. `dir.is_dir()`
    /// follows symlinks, so a link whose target held such a `.git`
    /// passed, and `remove_dir_all` then deleted the TARGET's contents.
    /// The path was never canonicalised and never compared to anything,
    /// and `remove_orphan` is exposed on the remote surface as
    /// `Class::Destructive`, so it could arrive from a paired peer.
    ///
    /// Both rules were already written down twice, which is the whole
    /// point of this guard. `artifacts::remove_artifact` and
    /// `caches::remove_venv` each carry a paragraph on why the symlink
    /// check must precede `canonicalize`, and `commands::remove_artifacts`
    /// states the containment rule outright: *"containment is the only
    /// thing between a bad path and `remove_dir_all` on an arbitrary
    /// directory, so the boundary it checks against must come from
    /// settings, not from the request."* `remove_orphan` sat one file
    /// away from both and had neither.
    ///
    /// # What it cannot see
    ///
    /// - **Whether the checks are right**, only that they are there. A
    ///   `symlink_metadata` whose answer is discarded passes. The three
    ///   gates' own behavioural tests cover that --
    ///   `refuses_a_symlink_pointing_inside_the_root`,
    ///   `refuses_a_traversal_out_of_the_root` and their siblings -- and
    ///   this guard is what stops a FOURTH gate shipping without them.
    /// - **A check in a helper.** `remove_venv` delegates containment to
    ///   `is_inside_cache`, so containment is recognised by any of several
    ///   spellings rather than one. That breadth is deliberate and is the
    ///   reason the symlink half is asserted separately and strictly: it
    ///   has no helper form in this codebase.
    /// - **A deletion that is not `remove_dir_all`.** A hand-rolled
    ///   recursive walk calling `remove_file`, or `Command::new("rm")`,
    ///   is outside this check. Nothing in the tree does either today.
    #[test]
    fn every_recursive_delete_checks_symlinks_and_containment() {
        let mut checked = 0usize;
        for (crate_name, root) in crate_roots() {
            for file in rust_files(&root) {
                let Ok(src) = std::fs::read_to_string(&file) else {
                    continue;
                };
                let prod = &production(&src);
                let rel = file.strip_prefix(&root).unwrap_or(&file).display();
                let mut at = 0usize;
                while let Some(i) = prod[at..].find("remove_dir_all(") {
                    let hit = at + i;
                    at = hit + 1;
                    // The line it sits on, so a doc comment explaining
                    // the rule is not mistaken for a call that breaks it.
                    let line_start = prod[..hit].rfind('\n').map_or(0, |j| j + 1);
                    let line_end = prod[hit..].find('\n').map_or(prod.len(), |j| hit + j);
                    let line = &prod[line_start..line_end];
                    if is_comment(line) {
                        continue;
                    }
                    let (fn_name, body) = enclosing_fn(prod, hit);
                    checked += 1;

                    assert!(
                        body.contains("symlink_metadata") && body.contains("is_symlink"),
                        "{crate_name}/{rel}: `{fn_name}` calls remove_dir_all with no symlink \
                         check. `remove_dir_all` on a symlink deletes the TARGET's contents, \
                         which may be anywhere at all -- and `is_dir()` follows links, so it \
                         sees the target's type and not the link. Call `symlink_metadata` and \
                         reject `is_symlink()` BEFORE `canonicalize`, which resolves through \
                         links and leaves nothing to detect. `artifacts::remove_artifact` and \
                         `caches::remove_venv` both document this at length; \
                         `worktrees::remove_orphan` is the sibling that did not get it (#854).\n\
                         \x20   {}",
                        line.trim()
                    );

                    // Containment, in any of the spellings this codebase
                    // uses. Broad on purpose: `remove_venv` delegates to
                    // `is_inside_cache`, so demanding a literal
                    // `starts_with` here would fail a gate that is
                    // correct. What every spelling has in common is that
                    // the canonical path is compared against a boundary.
                    let contains = body.contains("starts_with")
                        || body.contains("is_inside_cache")
                        || body.contains("outside the scanned folders");
                    assert!(
                        body.contains("canonicalize") && contains,
                        "{crate_name}/{rel}: `{fn_name}` calls remove_dir_all without checking \
                         the CANONICAL path is inside a root it was given. Without it the \
                         command is `remove_dir_all` on any directory its caller names, and \
                         `..` walks out of any root compared before canonicalising. The \
                         boundary must come from settings rather than from the request -- \
                         `commands::remove_artifacts` states exactly that, and \
                         `commands::remove_orphan` passed the path alone until #854.\n\
                         \x20   {}",
                        line.trim()
                    );
                }
            }
        }
        // The scan is asserted to have FOUND something, so a rename or a
        // moved file fails loudly rather than passing vacuously over an
        // empty list -- which is how a derived guard dies quietly.
        // `every_stats_query_meters_itself` guards itself the same way.
        assert!(
            checked >= 3,
            "only {checked} production remove_dir_all call(s) found; the scan is broken, \
             not the code. There are three (artifacts, caches, worktrees)."
        );
    }

    // ---- Invariant 2: refs reaching a git argv ---------------------------

    /// Every reader of `refs/remotes/origin/HEAD` validates what it got.
    ///
    /// # What it enforces
    ///
    /// A production function that asks git for `refs/remotes/origin/HEAD`
    /// must pass the answer through `is_safe_ref` before returning it.
    ///
    /// # Why this shape, and not the one #854 asked for
    ///
    /// #854 states the invariant as "every ref or name reaching a
    /// `git`/`docker` argv is behind a flag-shape validator or a `--`".
    /// That was written as a PER-SINK rule, and this codebase deliberately
    /// rejected the per-sink form. `is_safe_ref`'s own doc comment says
    /// so: validation is done "at the BOUNDARIES where remote-controlled
    /// refs enter ... rather than at each of the ten call sites, because a
    /// boundary cannot be forgotten. The `--` separators at the sinks are
    /// the second layer, not the only one."
    ///
    /// A guard demanding a validator or a `--` at each of roughly forty
    /// `git` spawn sites would therefore report the architecture as the
    /// defect. It would fire on `merge-base --is-ancestor HEAD <default>`,
    /// which is correct by construction -- the ref was validated at its
    /// boundary and git's own `merge-base` takes no `--` -- and on
    /// `config --get branch.<b>.remote`, where the name is embedded in a
    /// prefix and cannot be read as a flag at all. Dozens of findings,
    /// none of them real. That is the unanchored-pattern mistake
    /// `check-privacy.sh:120` records forty false positives from, and a
    /// gate that cries wolf is a gate someone disables.
    ///
    /// So the invariant is asserted where the codebase actually places it:
    /// at the boundary. The property "a remote-controlled ref is validated
    /// as it enters" is the one that makes every downstream sink safe, and
    /// it is checkable precisely because the boundaries are few and
    /// identifiable by the ref they read.
    ///
    /// # The finding it catches (#854)
    ///
    /// FOUR functions named `default_branch` read
    /// `refs/remotes/origin/HEAD`, and before #854 exactly ONE validated
    /// it -- `worktrees::scan::default_branch`, which carries the
    /// reasoning in its own comment: *"`origin/HEAD` is written by the
    /// remote, so the short name it yields is validated BEFORE the
    /// `origin/` prefix is put back on. Prefixing first would hide
    /// `--output=EVIL` behind a name that no longer starts with `-`."*
    ///
    /// `branches::scan`, `packages::apply` and `docker::classify` each
    /// grew their own and returned the name unvalidated. It then reached
    /// `rev-list <default>`, `merge-base <branch> <default>` and
    /// `merge-base --is-ancestor <tag> <default>` as a bare argv element
    /// with no `--`, where `--output=/path` is an arbitrary file write
    /// with the app's privileges.
    ///
    /// The reach of the validator was the mechanism of the failure:
    /// `is_safe_ref` was `pub(super)`, so the other three modules could
    /// not call it even had they wanted to. A shared rule only one module
    /// can see is a rule with one user.
    ///
    /// # What it cannot see
    ///
    /// - **Every other remote-controlled value.** This asserts ONE
    ///   boundary, the one with four implementations and three defects.
    ///   `parse_porcelain`'s branch names are the other, guarded and
    ///   tested since it was written. A tag read from a docker image
    ///   label (`docker::origin`) is a third, gated by `looks_like_sha`
    ///   instead -- a stricter check, not a missing one.
    /// - **Whether the validation is correctly ORDERED** -- only that it
    ///   is ordered at all. `symbolic-ref --short` returns
    ///   `origin/<name>`, so `is_safe_ref` on the PREFIXED string is
    ///   worthless: `origin/--output=/tmp/x` begins with `o`. This is not
    ///   hypothetical. Two of the three fixes #854 wrote made exactly that
    ///   mistake, and this guard passed on both -- it was the behavioural
    ///   test `a_flag_shaped_remote_head_is_refused` that caught them. So
    ///   the assertion below also requires a prefix strip in the same
    ///   function, which narrows the hole without closing it: a strip of
    ///   the wrong prefix, or of the right one applied to the wrong value,
    ///   still passes. The per-site behavioural tests are what cover that,
    ///   and they are named here so the pairing is not accidental.
    /// - **The sinks themselves.** A new `git` call passing an
    ///   unvalidated ref from somewhere else entirely is outside this.
    ///   That is the per-sink question, and it is the one judged
    ///   unenforceable above.
    #[test]
    fn every_reader_of_the_remote_head_validates_it() {
        const REF: &str = "refs/remotes/origin/HEAD";
        let mut checked = 0usize;
        for (crate_name, root) in crate_roots() {
            for file in rust_files(&root) {
                let Ok(src) = std::fs::read_to_string(&file) else {
                    continue;
                };
                let prod = &production(&src);
                let rel = file.strip_prefix(&root).unwrap_or(&file).display();
                let mut at = 0usize;
                while let Some(i) = prod[at..].find(REF) {
                    let hit = at + i;
                    at = hit + 1;
                    let line_start = prod[..hit].rfind('\n').map_or(0, |j| j + 1);
                    let line_end = prod[hit..].find('\n').map_or(prod.len(), |j| hit + j);
                    let line = &prod[line_start..line_end];
                    if is_comment(line) {
                        continue;
                    }
                    let (fn_name, body) = enclosing_fn(prod, hit);
                    checked += 1;
                    assert!(
                        body.contains("is_safe_ref"),
                        "{crate_name}/{rel}: `{fn_name}` reads {REF} and never passes the \
                         answer through `is_safe_ref`. That symref is written by the \
                         REMOTE, so the name it yields is remote-controlled -- and git \
                         ref names may legitimately begin with `-`. Passed as a bare \
                         argv element, `--output=/path` makes `git log` write to an \
                         arbitrary file, with this app's privileges. \
                         `worktrees::scan::default_branch` has validated it since it \
                         was written and states why; three siblings did not, because \
                         the validator was `pub(super)` (#854). Validate the BARE name, \
                         before any `origin/` prefix is put back on.\n\x20   {}",
                        line.trim()
                    );
                    // And the name validated must be the BARE one.
                    //
                    // `--short` returns `origin/<name>`, so `is_safe_ref`
                    // on that string is worthless -- `origin/--output=/x`
                    // begins with `o`. Two of #854's own three fixes made
                    // this mistake and this guard passed both until the
                    // check was added, so it is asserted rather than
                    // trusted to review.
                    assert!(
                        body.contains("strip_prefix(\"origin/\")") || body.contains("rsplit('/')"),
                        "{crate_name}/{rel}: `{fn_name}` calls `is_safe_ref` but never \
                         strips the `origin/` prefix, so it is validating a string that \
                         starts with `o` whatever the remote named the branch. \
                         `symbolic-ref --short` returns `origin/<name>`; validate \
                         `<name>` (#854)."
                    );
                }
            }
        }
        // Guards the guard: four readers exist, and a scan that found
        // fewer has stopped seeing one of them.
        assert!(
            checked >= 4,
            "only {checked} reader(s) of {REF} found; the scan is broken, not the \
             code. There are four (worktrees, branches, packages, docker)."
        );
    }

    // ---- Invariant 5: mirrored constants ---------------------------------

    /// A constant declared in more than one crate has a test that reads
    /// both copies.
    ///
    /// # What it enforces
    ///
    /// Every `const NAME` that appears in the production half of two
    /// different crates must either be asserted by a test that reads both
    /// sides, or appear in `COINCIDENTAL` below with a reason.
    ///
    /// # The finding it would have caught (#850)
    ///
    /// #850 found five pairs whose doc comments said the agreement was
    /// asserted and which no test checked; one had already drifted --
    /// `15 * 60` in Rust against `60 * 60` in the UI, both comments
    /// claiming they matched, with three user-visible consequences. The
    /// remedy, `src/lib/mirroredConstants.test.ts`, is excellent and
    /// ENUMERATED: it covers exactly the five pairs that audit found.
    ///
    /// So this asks the general question instead, and the answer is that
    /// the Rust-to-Rust pairs were never in scope of that file at all.
    /// Worst among them, and the reason this is worth a guard rather than
    /// five more assertions: `ECDSA_SIG_LEN` and `MLDSA_SIG_LEN` are
    /// declared in `crates/headstate-stepup` -- whose module doc says in
    /// so many words that it holds what "both ends must agree on" -- and
    /// then DECLARED AGAIN in `src-mobile/src/keys.rs` rather than
    /// imported from it. `keys.rs` then asserts against its own local
    /// copies, so a change to the shared crate is invisible on the phone.
    /// These are signature lengths on a security boundary.
    ///
    /// # Why the pairs are derived structurally, not from the prose
    ///
    /// The obvious derivation is to grep doc comments for "must match" /
    /// "mirrors" and demand a test per hit. It was tried and rejected:
    /// over this tree that phrase matches mostly ordinary prose
    /// ("singular and plural must agree with the number", "the default
    /// branch must agree"), which is the unanchored-pattern mistake
    /// `check-privacy.sh` records ~40 false positives from. A repeated
    /// NAME is a fact about the code rather than about how carefully
    /// somebody worded a comment, and it also catches the pair whose
    /// comment says nothing at all -- which `VALIDITY_YEARS` very nearly
    /// is.
    ///
    /// # What it cannot see
    ///
    /// - **A pair with different names on each side.** `SEED_LEN` (32) in
    ///   the desktop and `VAULT_KEY_LEN` (32) on the phone are the same
    ///   32 bytes and this cannot tell. Name-matching is the price of not
    ///   matching on prose.
    /// - **A value duplicated as a bare literal** rather than as a named
    ///   constant. `PORT` is `41919` in the desktop and an unnamed
    ///   `41919` five times over on the phone, and only the named side is
    ///   visible here.
    /// - **Whether the covering test is any GOOD.** It checks that a test
    ///   names the constant and reads both files, not that it compares
    ///   them correctly. `mirroredConstants.test.ts` guards its own
    ///   extractor for this reason, and `NONCE_LEN` is the live example
    ///   of a test that names a constant and still reads one side twice
    ///   (`assert_eq!(NONCE_LEN, 16)`).
    /// - **TypeScript.** The Rust-to-TS pairs are
    ///   `mirroredConstants.test.ts`' business and stay there; this is
    ///   its Rust-to-Rust counterpart, not its replacement.
    #[test]
    fn every_cross_crate_constant_is_read_from_both_sides() {
        /// Names that collide by coincidence rather than by mirroring.
        ///
        /// An explicit, reviewed list with a reason per entry, which is
        /// the form this codebase insists on over a looser pattern:
        /// `surfaceGuard.test.ts`' `DESKTOP_ONLY_WRAPPERS` makes the same
        /// argument, that adding to such a list should be a deliberate
        /// act somebody reads.
        ///
        /// Every entry here is a name two crates use for DIFFERENT
        /// things, verified by reading both. A pair that merely happens
        /// to agree today does not belong here -- that is the thing being
        /// guarded.
        const COINCIDENTAL: &[(&str, &str)] = &[
            // 120s for a companion's whole HTTP call to the desktop,
            // 20s for one `docker` subprocess. Unrelated budgets that
            // would be wrong to tie together.
            (
                "CALL_TIMEOUT",
                "unrelated timeouts: an HTTP call vs a docker subprocess",
            ),
            // The phone's key record is at schema 1, the desktop's
            // identity record at 2. Separate formats on separate
            // migration paths; forcing them equal would be meaningless.
            (
                "STORED_VERSION",
                "independent on-disk record schemas, versioned separately",
            ),
            // The desktop's is the mDNS TXT string `"1"`, the phone's the
            // integer 1 in its own pairing record. Different types,
            // different records.
            (
                "RECORD_VERSION",
                "a TXT string on one side, a record schema integer on the other",
            ),
        ];

        let mut by_name: BTreeMap<String, Vec<(String, String)>> = BTreeMap::new();
        for (crate_name, root) in crate_roots() {
            for file in rust_files(&root) {
                let Ok(src) = std::fs::read_to_string(&file) else {
                    continue;
                };
                for line in production(&src).lines() {
                    if is_comment(line) {
                        continue;
                    }
                    // `const NAME:` at any indentation, with or without
                    // `pub`. SCREAMING_CASE only, which is what
                    // distinguishes a constant from a local binding.
                    let t = line.trim_start();
                    let rest = t
                        .strip_prefix("pub const ")
                        .or_else(|| t.strip_prefix("const "))
                        .or_else(|| {
                            t.strip_prefix("pub(crate) const ")
                                .or_else(|| t.strip_prefix("pub(super) const "))
                        });
                    let Some(rest) = rest else { continue };
                    let name: String = rest
                        .chars()
                        .take_while(|c| c.is_ascii_uppercase() || c.is_ascii_digit() || *c == '_')
                        .collect();
                    if name.len() < 3 || !rest[name.len()..].trim_start().starts_with(':') {
                        continue;
                    }
                    let file_name = file
                        .file_name()
                        .map(|n| n.to_string_lossy().into_owned())
                        .unwrap_or_default();
                    by_name
                        .entry(name)
                        .or_default()
                        .push((crate_name.to_string(), file_name));
                }
            }
        }

        // Every FUNCTION that reads another file's source as text, as a
        // list of its bodies. A covering assertion has to be inside one
        // of these, because reading the other side's source is the only
        // way an assertion can fail when the other side alone changes --
        // which is the whole property. `mirroredConstants.test.ts`' own
        // header makes the argument: a test comparing a constant to a
        // literal reads ONE side twice and passes at any value.
        //
        // Scoped to the function rather than to the file for the reason
        // the first version of this check got wrong: a file-wide search
        // for `include_str!` matched every file in the tree, so the
        // condition was true for every constant and the whole assertion
        // was vacuous. It passed green over sixteen uncovered pairs.
        let mut readers: Vec<String> = Vec::new();
        for (_, root) in crate_roots() {
            for file in rust_files(&root) {
                let Ok(src) = std::fs::read_to_string(&file) else {
                    continue;
                };
                let mut at = 0usize;
                while let Some(i) = src[at..].find("include_str!") {
                    let hit = at + i;
                    at = hit + 1;
                    // Only a read of a DIFFERENT crate's source. A file
                    // reading its own text (`include_str!("scan.rs")`)
                    // is a self-scan, not a mirror.
                    let line_end = src[hit..].find('\n').map_or(src.len(), |j| hit + j);
                    if !src[hit..line_end].contains("..") {
                        continue;
                    }
                    readers.push(enclosing_fn(&src, hit).1);
                }
            }
        }
        // The frontend's mirror test is one reader too: it is where a
        // Rust-to-TypeScript pair's assertion belongs, and a constant
        // asserted there is covered.
        let ts = Path::new(env!("CARGO_MANIFEST_DIR")).join("../src/lib/mirroredConstants.test.ts");
        if let Ok(s) = std::fs::read_to_string(&ts) {
            readers.push(s);
        }
        assert!(
            readers.len() >= 2,
            "only {} cross-file source reader(s) found; the scan is broken. \
             `src-mobile/src/surface.rs` and `src/lib/mirroredConstants.test.ts` \
             are two of them.",
            readers.len()
        );

        let mut pairs = 0usize;
        let mut unguarded = Vec::new();
        for (name, sites) in &by_name {
            let crates: std::collections::BTreeSet<&str> =
                sites.iter().map(|(c, _)| c.as_str()).collect();
            if crates.len() < 2 {
                continue;
            }
            pairs += 1;
            if COINCIDENTAL.iter().any(|(n, _)| *n == name) {
                continue;
            }
            // Covered when some function that reads another file's
            // source also NAMES this constant. Both halves in the same
            // scope is the point: naming it without reading across is
            // what `top_n_is_five` did while only ever seeing Rust, and
            // reading across without naming it says nothing about this
            // constant.
            let covered = readers.iter().any(|body| body.contains(name.as_str()));
            if !covered {
                let where_ = sites
                    .iter()
                    .map(|(c, f)| format!("{c}/{f}"))
                    .collect::<Vec<_>>()
                    .join(", ");
                unguarded.push(format!("{name} ({where_})"));
            }
        }

        // Guards the guard, both directions. A pattern that stopped
        // matching would report every pair as covered; one that matched
        // nothing would report no pairs at all.
        assert!(
            pairs >= 10,
            "only {pairs} cross-crate constant(s) found; the scan is broken, not the code"
        );
        assert!(
            unguarded.is_empty(),
            "these constants are declared in two crates and no test reads both copies:\n  {}\n\n\
             A doc comment saying two values must agree cannot enforce itself -- #850 found \
             five such comments and one pair had already drifted while both went on claiming \
             they matched. Add an assertion that reads the OTHER side's SOURCE \
             (`include_str!` in Rust, `?raw` from TypeScript), the way \
             `src-mobile/src/surface.rs` reads the desktop's table and \
             `src/lib/mirroredConstants.test.ts` reads Rust constants. Asserting one copy \
             against a literal reads one side twice and cannot fail when the other moves. \
             If the collision is coincidental, add it to COINCIDENTAL above with a reason \
             (#854).",
            unguarded.join("\n  ")
        );
    }
}
