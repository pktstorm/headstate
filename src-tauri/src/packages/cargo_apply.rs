//! Applying a Cargo update to the manifest that actually declares it.
//!
//! The write half of `packages::cargo`, and the first place in this app
//! that edits a dependency file directly rather than asking a package
//! manager to. That is a deliberate reversal of the rule everywhere
//! else here, so it is worth saying why.
//!
//! ## Why not `cargo add`
//!
//! #560 measured `cargo add` and refused to ship an apply button on the
//! strength of what it found. Re-measured here against cargo 1.98.1
//! before any of this was designed, because a fix built on a stale
//! measurement is a fix for nothing. All three still hold:
//!
//! 1. **It severs workspace inheritance.** A member declaring
//!    `thiserror.workspace = true` becomes `thiserror = "2.0.18"` --
//!    the inheritance gone, the why-comment above it deleted, and the
//!    root's `[workspace.dependencies]` left stale on the old version.
//!    `cargo add` DOES have `--package`, contrary to what #559 assumed,
//!    and aiming it correctly does not help: the rewrite happens in the
//!    member either way. There is no flag that says "update the
//!    workspace entry this inherits from".
//!
//! 2. **Unaimed, it edits the wrong crate.** At a non-virtual root -- a
//!    package that is also a workspace root, which is `src-mobile`'s
//!    exact shape -- asking for a crate only a MEMBER declares adds a
//!    new dependency to the root package and leaves the member alone.
//!    This one `--package` does fix, and it is the only one of the
//!    three it fixes.
//!
//! 3. **It flattens the constraint style.** Newly measured, and not in
//!    #560. `serde = "1"` becomes `serde = "1.0.229"`, narrowing a
//!    deliberately broad requirement to a near-pin; `log = "=0.4.20"`
//!    becomes `log = "0.4.30"`, converting a deliberate EXACT PIN into
//!    a caret range. Both directions are a rewrite of the user's
//!    version policy, which is the mistake #409 went to some trouble to
//!    avoid for npm -- there, by passing `--save-exact` only when the
//!    manifest was already exact.
//!
//! `cargo add` is built on `toml_edit` and does preserve comments and
//! features. So rather than fight a tool whose defaults are three
//! separate wrong edits, this uses the same library directly and makes
//! the three decisions itself.
//!
//! ## What it does not do
//!
//! It does not touch `Cargo.lock`. See `LOCKFILE` below.

use super::cargo::{self, Declared, Source, Table};
use std::path::{Path, PathBuf};
use toml_edit::{DocumentMut, Item, Value};

/// Why the lockfile is left alone.
///
/// `Cargo.toml` and `Cargo.lock` disagree after this runs, and that is
/// the chosen outcome rather than an oversight.
///
/// Reconciling them means `cargo update -p <crate> --precise <version>`,
/// which needs the `cargo` binary. Every other apply path here spawns
/// its ecosystem's tool and fails loudly when it is missing -- but
/// Cargo was deliberately built as one of the three ecosystems that
/// needs NO tool installed (`model::Ecosystem::Cargo`), for the reason
/// `cargo.rs` states at length: a missing binary is what turns a report
/// into a confident empty list. Making APPLY depend on a binary that
/// CHECK refuses to depend on would put the two halves of one ecosystem
/// on different footings.
///
/// The stale lock is also visible and self-correcting in a way a wrong
/// manifest edit is not: the next `cargo build`, `cargo test` or `cargo
/// check` updates it automatically, and CI on the pull request does
/// exactly that. A manifest edited to the wrong version, by contrast,
/// is silent.
///
/// So the manifest edit is the whole job, and the pull request body
/// says the lockfile is untouched rather than leaving it to be
/// discovered.
pub const LOCKFILE: &str = "Cargo.lock is not updated; the next cargo build refreshes it.";

/// The manifest an update must edit, and the entry inside it.
///
/// Two paths rather than one, because for an inherited dependency they
/// differ: the entry the user sees lives in the member, and the version
/// it resolves against lives in the root. Editing the former is the
/// failure this module exists to prevent.
#[derive(Debug, Clone, PartialEq)]
pub struct Target {
    /// The file to write.
    pub manifest: PathBuf,
    /// The dotted path to the entry within that file, for the report.
    pub table: String,
    /// The manifest key, which is not always the crate name.
    pub key: String,
    /// Whether this resolved through `workspace = true`.
    pub inherited: bool,
}

/// Which manifest declares this crate, in this project.
///
/// Reuses `cargo::declared` rather than walking the workspace again.
/// The report the user is acting on came from that function, and a
/// second walk with its own opinion about members and globs could
/// resolve to a file the report never named -- which is the whole class
/// of bug this issue is about.
///
/// `table` and `target` narrow the match because one crate legitimately
/// appears several times: both `src-mobile` plugins declare `tauri`
/// plainly and again under `[target.'cfg(target_os = "ios")'
/// .dependencies]`, and those are two entries in two places. When the
/// caller cannot say which -- the wizard sends a name and a version --
/// the FIRST in `declared` order is taken, matching the row the page
/// showed, and the returned `Target` names the file so the report can
/// say which one was edited.
pub fn resolve(project: &Path, name: &str) -> Result<Target, String> {
    let all = cargo::declared(project);
    let d = all
        .iter()
        .find(|d| d.name == name)
        .ok_or_else(|| format!("{name} is not declared in any manifest of this workspace"))?;

    // A path, git or alternative-registry dependency has no crates.io
    // version to move to, and the page already reports it as
    // uncomparable. Refused here as well rather than trusted to be
    // filtered upstream: this is the function that would write the file.
    match d.source {
        Source::Registry => {}
        Source::Path => {
            return Err(format!(
                "{name} is a path dependency; there is no published version to move to"
            ));
        }
        Source::Git => {
            return Err(format!("{name} is a git dependency; its version comes from the checkout, not from crates.io"));
        }
        Source::Other => {
            return Err(format!(
                "{name} comes from an alternative registry, which is not checked here"
            ));
        }
    }

    Ok(target_for(project, d))
}

/// The `Target` for one already-resolved `Declared`.
///
/// Split out so the inheritance rule can be tested directly on a
/// `Declared` without a workspace on disk.
fn target_for(project: &Path, d: &Declared) -> Target {
    if d.inherited {
        // THE RULE. An inherited entry's version lives in the root's
        // `[workspace.dependencies]` and nowhere else. Writing a
        // version onto the member's `foo.workspace = true` does not
        // update it -- it replaces the inheritance with a hardcoded
        // number, which is measured failure 1.
        return Target {
            manifest: project.join("Cargo.toml"),
            table: "workspace.dependencies".to_string(),
            key: d.key.clone(),
            inherited: true,
        };
    }
    Target {
        manifest: d.manifest.clone(),
        table: table_path(d),
        key: d.key.clone(),
        inherited: false,
    }
}

/// The dotted table path for a non-inherited entry.
fn table_path(d: &Declared) -> String {
    let table = match d.table {
        Table::Dependencies => "dependencies",
        Table::Dev => "dev-dependencies",
        Table::Build => "build-dependencies",
    };
    match &d.target {
        Some(t) => format!("target.'{t}'.{table}"),
        None => table.to_string(),
    }
}

/// Rewrite one version requirement, keeping the style the user chose.
///
/// The #409 rule, carried over from npm and applied to Cargo's grammar
/// rather than to semver ranges. The principle is the same one that
/// motivated `--save-exact`: a constraint is a POLICY, and an update is
/// not permission to change it.
///
/// Cargo's default requirement is caret -- `"1.2.3"` and `"^1.2.3"`
/// mean the same thing -- so the shapes that carry intent are:
///
/// - `"=1.2.3"` an exact pin. Stays exact: `"=1.2.4"`. `cargo add`
///   drops the `=` and silently unpins the project. Measured.
/// - `"^1.2.3"` an explicit caret. Keeps the `^`, because the user
///   wrote it and a diff that removes it is noise at best.
/// - `"1.2.3"` the bare default. Stays bare.
/// - `"1"` or `"1.2"` a deliberately BROAD requirement. This is the
///   subtle one. `1` already admits `1.0.229`, so an update to
///   `1.0.229` needs NO EDIT AT ALL -- and rewriting it to `"1.0.229"`
///   narrows a policy the user chose. `cargo add` rewrites it.
///   Returning `None` here means "the constraint already covers this",
///   and the caller reports the entry as needing no change rather than
///   writing one.
/// - `">=1, <2"`, `"~1.2"`, `"1.*"` and anything else with an operator
///   this does not model. Refused rather than guessed at: a compound
///   range rewritten by a rule that does not understand it is exactly
///   the confidently-wrong edit this module exists to avoid.
///
/// `None` with `covered` true means no edit is needed; the caller
/// distinguishes the two through `Rewrite`.
#[derive(Debug, Clone, PartialEq)]
pub enum Rewrite {
    /// Write this string in place of the old one.
    To(String),
    /// The existing constraint already admits the new version, so
    /// editing it would only narrow the user's policy.
    AlreadyCovered,
    /// A constraint shape this does not model. Never guessed at.
    Unsupported(String),
}

pub fn rewrite(current: &str, version: &str) -> Rewrite {
    let c = current.trim();
    if c.is_empty() {
        return Rewrite::Unsupported("the manifest holds an empty version requirement".into());
    }
    // A comma is a compound requirement (`>= 1.2, < 2`). Modelling one
    // means deciding which half to move and by how much, and there is
    // no answer that is right in general.
    if c.contains(',') {
        return Rewrite::Unsupported(format!(
            "{c:?} is a compound requirement; move it by hand so the intent is kept"
        ));
    }
    // A wildcard admits far more than a caret and is not a shape an
    // exact version can replace without narrowing it.
    if c.contains('*') {
        return Rewrite::Unsupported(format!("{c:?} is a wildcard requirement"));
    }

    let (prefix, bare) = split_operator(c);
    match prefix {
        // Unmodelled operators. `~` and `>` change what the requirement
        // ADMITS in ways a version swap does not preserve.
        "~" | ">" | ">=" | "<" | "<=" => {
            return Rewrite::Unsupported(format!(
                "{c:?} uses the {prefix:?} operator, which this does not rewrite"
            ))
        }
        _ => {}
    }

    // A requirement shorter than the new version is a deliberately
    // broad one, and it may already admit the update. `1` admits
    // 1.0.229; `1.0` admits 1.0.229; `1.0.228` does not admit 1.0.229.
    //
    // Only for caret-semantics requirements. An EXACT pin of `=1` means
    // 1.0.0 and admits nothing else, so it is always rewritten.
    if prefix != "=" && admits(bare, version) {
        return Rewrite::AlreadyCovered;
    }

    Rewrite::To(format!("{prefix}{version}"))
}

/// The operator at the front of a requirement, and the rest.
fn split_operator(req: &str) -> (&str, &str) {
    for op in ["<=", ">=", "^", "~", "=", "<", ">"] {
        if let Some(rest) = req.strip_prefix(op) {
            return (op, rest.trim_start());
        }
    }
    ("", req)
}

/// Whether a caret requirement already admits a version.
///
/// Caret is the default, and it admits anything that does not change
/// the leftmost NON-ZERO component. Rather than reimplement semver, the
/// only question asked here is the narrow one that matters: is the
/// requirement a strict PREFIX of the new version at a component
/// boundary? `1` is a prefix of `1.0.229`, `1.0` is a prefix of
/// `1.0.229`, `1.0.228` is not a prefix of `1.0.229`.
///
/// Deliberately conservative in the direction of editing: a requirement
/// this cannot prove already covers the version gets rewritten, which
/// is the visible outcome rather than the silent one.
fn admits(req: &str, version: &str) -> bool {
    if req == version {
        return true;
    }
    let req_parts: Vec<&str> = req.split('.').collect();
    let ver_parts: Vec<&str> = version.split('.').collect();
    if req_parts.len() >= ver_parts.len() {
        return false;
    }
    // A leading zero component means caret narrows to that component
    // (`0.4` admits 0.4.x but not 0.5.0), which prefix matching already
    // gets right -- the check is the same either way.
    req_parts.iter().zip(ver_parts.iter()).all(|(r, v)| r == v)
}

/// What one manifest edit did.
#[derive(Debug, Clone, PartialEq)]
pub struct Edited {
    /// The file written, relative to the project when it could be made
    /// relative.
    pub manifest: String,
    /// The table the entry sits in.
    pub table: String,
    /// The requirement before.
    pub before: String,
    /// The requirement after. Equal to `before` when nothing was
    /// written.
    pub after: String,
    /// False when the existing constraint already admitted the version,
    /// so nothing was written. A real and reportable outcome, not a
    /// failure.
    pub written: bool,
}

/// Apply one Cargo update inside `project`.
///
/// `project` is the workspace root as `detect::projects` reports it,
/// inside a throwaway worktree. Nothing here reaches outside it.
///
/// Reads the manifest back after writing, the way every other apply
/// path does -- the file on disk is the authority on what landed, not
/// the string this function meant to write.
pub fn apply(project: &Path, name: &str, version: &str) -> Result<Edited, String> {
    let target = resolve(project, name)?;
    let text = std::fs::read_to_string(&target.manifest)
        .map_err(|e| format!("could not read {}: {e}", target.manifest.display()))?;
    let mut doc: DocumentMut = text
        .parse()
        .map_err(|e| format!("could not parse {}: {e}", target.manifest.display()))?;

    let item = entry(&mut doc, &target).ok_or_else(|| {
        format!(
            "{} has no [{}] entry for {}",
            target.manifest.display(),
            target.table,
            target.key
        )
    })?;

    let before =
        requirement(item).ok_or_else(|| format!("{} declares no version to move", target.key))?;

    let after = match rewrite(&before, version) {
        Rewrite::To(s) => s,
        Rewrite::AlreadyCovered => {
            return Ok(Edited {
                manifest: relative(project, &target.manifest),
                table: target.table,
                before: before.clone(),
                after: before,
                written: false,
            })
        }
        Rewrite::Unsupported(why) => return Err(why),
    };

    set_requirement(item, &after)?;

    std::fs::write(&target.manifest, doc.to_string())
        .map_err(|e| format!("could not write {}: {e}", target.manifest.display()))?;

    // Read back from DISK, not from the document in memory. The point
    // of the re-read everywhere else in `apply` is that the file is the
    // authority; a value echoed out of the object that wrote it proves
    // nothing about what landed.
    let confirmed = std::fs::read_to_string(&target.manifest)
        .ok()
        .and_then(|t| t.parse::<DocumentMut>().ok())
        .and_then(|mut d| entry(&mut d, &target).map(|i| &*i).and_then(requirement))
        .unwrap_or_else(|| after.clone());

    Ok(Edited {
        manifest: relative(project, &target.manifest),
        table: target.table,
        before,
        after: confirmed,
        written: true,
    })
}

/// The item for one dependency entry, walking the dotted table path.
fn entry<'a>(doc: &'a mut DocumentMut, target: &Target) -> Option<&'a mut Item> {
    let mut item: &mut Item = doc.as_item_mut();
    for part in table_parts(&target.table) {
        item = item.get_mut(&part)?;
    }
    item.get_mut(&target.key)
}

/// Split a dotted table path, keeping a quoted component whole.
///
/// `target.'cfg(target_os = "ios")'.dependencies` is three components,
/// and the middle one contains both dots and quotes. Splitting naively
/// on `.` would produce five nonsense components and find nothing.
fn table_parts(path: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut current = String::new();
    let mut quote: Option<char> = None;
    for c in path.chars() {
        match quote {
            Some(q) => {
                if c == q {
                    quote = None;
                } else {
                    current.push(c);
                }
            }
            None => match c {
                '\'' | '"' => quote = Some(c),
                '.' => {
                    out.push(std::mem::take(&mut current));
                }
                _ => current.push(c),
            },
        }
    }
    out.push(current);
    out.into_iter().filter(|s| !s.is_empty()).collect()
}

/// The version requirement an entry holds, in either form.
fn requirement(item: &Item) -> Option<String> {
    if let Some(s) = item.as_str() {
        return Some(s.to_string());
    }
    // A table entry: `{ version = "1", features = [...] }`, or the
    // dotted `foo.version = "1"` which parses the same way.
    item.get("version")?.as_str().map(str::to_string)
}

/// Write a requirement back in whichever form the entry already uses.
///
/// The form is NEVER changed. A bare string stays a bare string; a
/// table keeps its features, its `optional`, its `default-features` and
/// -- the part that matters most here -- the decoration `toml_edit`
/// carries, which is where the comment above the entry lives.
fn set_requirement(item: &mut Item, version: &str) -> Result<(), String> {
    if item.is_str() {
        // Replacing the VALUE only. `toml_edit` keeps the key's
        // decoration -- the whitespace and comments preceding it -- on
        // the key, so the why-comment above this line is untouched.
        let mut new = Value::from(version);
        // Carry the old value's own decoration (the spacing around the
        // `=` and any trailing same-line comment) so the line is
        // rewritten in place rather than reformatted.
        if let Some(old) = item.as_value() {
            *new.decor_mut() = old.decor().clone();
        }
        *item = Item::Value(new);
        return Ok(());
    }
    let Some(v) = item.get_mut("version") else {
        return Err("the entry has no version key to replace".into());
    };
    let mut new = Value::from(version);
    if let Some(old) = v.as_value() {
        *new.decor_mut() = old.decor().clone();
    }
    *v = Item::Value(new);
    Ok(())
}

/// A path relative to the project, for the report.
fn relative(project: &Path, manifest: &Path) -> String {
    manifest
        .strip_prefix(project)
        .unwrap_or(manifest)
        .to_string_lossy()
        .into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write(dir: &Path, rel: &str, body: &str) {
        let p = dir.join(rel);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, body).unwrap();
    }

    /// A virtual workspace root whose member INHERITS.
    ///
    /// The shape of measured failure 1.
    fn virtual_workspace(dir: &Path) {
        write(
            dir,
            "Cargo.toml",
            "[workspace]\n\
             members = [\"member\"]\n\
             \n\
             [workspace.dependencies]\n\
             # WHY: the inherited version lives here.\n\
             thiserror = \"2.0.17\"\n",
        );
        write(
            dir,
            "member/Cargo.toml",
            "[package]\n\
             name = \"member\"\n\
             version = \"0.1.0\"\n\
             \n\
             [dependencies]\n\
             # WHY: inherited from the workspace root.\n\
             thiserror.workspace = true\n",
        );
    }

    /// A package that is ALSO a workspace root, with members under
    /// `plugins/` -- `src-mobile`'s exact shape, and where measured
    /// failure 2 happens.
    fn non_virtual_workspace(dir: &Path) {
        write(
            dir,
            "Cargo.toml",
            "[package]\n\
             name = \"rootpkg\"\n\
             version = \"0.1.0\"\n\
             \n\
             [workspace]\n\
             members = [\"plugins/*\"]\n\
             \n\
             [dependencies]\n\
             # WHY: the root needs logging.\n\
             log = \"0.4.20\"\n",
        );
        write(
            dir,
            "plugins/thing/Cargo.toml",
            "[package]\n\
             name = \"thing\"\n\
             version = \"0.1.0\"\n\
             \n\
             [dependencies]\n\
             # WHY: only the member uses this.\n\
             thiserror = \"2.0.17\"\n",
        );
    }

    /// MEASURED FAILURE 1, as a regression test.
    ///
    /// `cargo add thiserror@2.0.18` at this root -- with or without
    /// `--package member` -- rewrites the member's
    /// `thiserror.workspace = true` into `thiserror = "2.0.18"`,
    /// severing the inheritance and deleting the comment above it,
    /// while leaving the root stale. None of that may happen here.
    #[test]
    fn an_inherited_dependency_updates_the_root_and_leaves_the_member_alone() {
        let dir = tempfile::tempdir().unwrap();
        virtual_workspace(dir.path());

        let e = apply(dir.path(), "thiserror", "2.0.18").unwrap();
        assert_eq!(e.manifest, "Cargo.toml", "the ROOT is the file to edit");
        assert_eq!(e.table, "workspace.dependencies");
        assert_eq!(e.after, "2.0.18");

        let root = std::fs::read_to_string(dir.path().join("Cargo.toml")).unwrap();
        assert!(
            root.contains("thiserror = \"2.0.18\""),
            "root moved: {root}"
        );
        assert!(
            root.contains("# WHY: the inherited version lives here."),
            "the root's why-comment survived: {root}"
        );

        let member = std::fs::read_to_string(dir.path().join("member/Cargo.toml")).unwrap();
        assert!(
            member.contains("thiserror.workspace = true"),
            "inheritance NOT severed: {member}"
        );
        assert!(
            !member.contains("2.0.18"),
            "no version hardcoded into the member: {member}"
        );
        assert!(
            member.contains("# WHY: inherited from the workspace root."),
            "the member's why-comment survived: {member}"
        );
    }

    /// MEASURED FAILURE 2, as a regression test.
    ///
    /// `cargo add thiserror@2.0.18` at this root adds a NEW dependency
    /// to the root package and leaves the member untouched. The right
    /// answer is the opposite of both halves.
    #[test]
    fn a_member_only_crate_updates_the_member_not_the_root_package() {
        let dir = tempfile::tempdir().unwrap();
        non_virtual_workspace(dir.path());

        let e = apply(dir.path(), "thiserror", "2.0.18").unwrap();
        assert_eq!(
            e.manifest, "plugins/thing/Cargo.toml",
            "the MEMBER is the file to edit"
        );

        let member = std::fs::read_to_string(dir.path().join("plugins/thing/Cargo.toml")).unwrap();
        assert!(member.contains("thiserror = \"2.0.18\""), "{member}");

        let root = std::fs::read_to_string(dir.path().join("Cargo.toml")).unwrap();
        assert!(
            !root.contains("thiserror"),
            "the root package gained NOTHING: {root}"
        );
        assert!(root.contains("log = \"0.4.20\""), "root untouched: {root}");
    }

    /// The reason #560 refused to ship a button.
    #[test]
    fn a_why_comment_above_a_dependency_survives() {
        let dir = tempfile::tempdir().unwrap();
        write(
            dir.path(),
            "Cargo.toml",
            "[package]\n\
             name = \"p\"\n\
             version = \"0.1.0\"\n\
             \n\
             [dependencies]\n\
             # WHY: this comment is the whole point of the issue.\n\
             # It runs to two lines, as several in this repo do.\n\
             thiserror = \"2.0.17\"\n\
             # WHY: the NEXT entry's comment must not move either.\n\
             log = \"0.4.20\"\n",
        );

        apply(dir.path(), "thiserror", "2.0.18").unwrap();

        let text = std::fs::read_to_string(dir.path().join("Cargo.toml")).unwrap();
        assert!(text.contains("# WHY: this comment is the whole point of the issue."));
        assert!(text.contains("# It runs to two lines, as several in this repo do."));
        assert!(text.contains("# WHY: the NEXT entry's comment must not move either."));
        assert!(text.contains("thiserror = \"2.0.18\""));
        assert!(
            text.contains("log = \"0.4.20\""),
            "the other entry is untouched"
        );
    }

    /// A table entry keeps its features. `cargo add` gets this right
    /// too; the test is here so a hand-rolled writer cannot regress it.
    #[test]
    fn a_table_entry_keeps_its_features_and_its_comment() {
        let dir = tempfile::tempdir().unwrap();
        write(
            dir.path(),
            "Cargo.toml",
            "[package]\n\
             name = \"p\"\n\
             version = \"0.1.0\"\n\
             \n\
             [dependencies]\n\
             # WHY: derive is needed for the config types.\n\
             serde = { version = \"1.0.200\", features = [\"derive\"], optional = true }\n",
        );

        apply(dir.path(), "serde", "1.0.229").unwrap();

        let text = std::fs::read_to_string(dir.path().join("Cargo.toml")).unwrap();
        assert!(text.contains("version = \"1.0.229\""), "{text}");
        assert!(text.contains("features = [\"derive\"]"), "{text}");
        assert!(text.contains("optional = true"), "{text}");
        assert!(text.contains("# WHY: derive is needed for the config types."));
    }

    /// A target-specific entry is found through the quoted table name.
    #[test]
    fn a_target_specific_entry_is_edited_in_place() {
        let dir = tempfile::tempdir().unwrap();
        write(
            dir.path(),
            "Cargo.toml",
            "[package]\n\
             name = \"p\"\n\
             version = \"0.1.0\"\n\
             \n\
             [target.'cfg(target_os = \"ios\")'.dependencies]\n\
             # WHY: iOS needs the mobile feature.\n\
             tauri = { version = \"2.0.0\", features = [\"mobile\"] }\n",
        );

        let e = apply(dir.path(), "tauri", "2.1.0").unwrap();
        assert_eq!(e.table, "target.'cfg(target_os = \"ios\")'.dependencies");

        let text = std::fs::read_to_string(dir.path().join("Cargo.toml")).unwrap();
        assert!(text.contains("version = \"2.1.0\""), "{text}");
        assert!(text.contains("features = [\"mobile\"]"), "{text}");
        assert!(text.contains("# WHY: iOS needs the mobile feature."));
    }

    /// #409's rule, in Cargo's grammar. An exact pin stays exact --
    /// `cargo add` drops the `=` and unpins the project. Measured.
    #[test]
    fn an_exact_pin_stays_exact() {
        assert_eq!(rewrite("=0.4.20", "0.4.30"), Rewrite::To("=0.4.30".into()));
    }

    /// An explicit caret keeps its caret, so the diff is one number.
    #[test]
    fn an_explicit_caret_keeps_its_caret() {
        assert_eq!(rewrite("^2.0.17", "2.0.18"), Rewrite::To("^2.0.18".into()));
    }

    /// The bare default stays bare.
    #[test]
    fn a_bare_requirement_stays_bare() {
        assert_eq!(rewrite("2.0.17", "2.0.18"), Rewrite::To("2.0.18".into()));
    }

    /// The subtle one. `serde = "1"` ALREADY admits 1.0.229, so an
    /// update needs no edit -- and `cargo add` narrows it to "1.0.229",
    /// rewriting a policy the user chose. Measured.
    #[test]
    fn a_broad_requirement_that_already_admits_the_version_is_not_narrowed() {
        assert_eq!(rewrite("1", "1.0.229"), Rewrite::AlreadyCovered);
        assert_eq!(rewrite("1.0", "1.0.229"), Rewrite::AlreadyCovered);
        // But a broad requirement that does NOT admit it still moves.
        assert_eq!(rewrite("1", "2.0.0"), Rewrite::To("2.0.0".into()));
        assert_eq!(rewrite("0.4", "0.5.1"), Rewrite::To("0.5.1".into()));
    }

    /// `=1` means exactly 1.0.0 and admits nothing else, so the
    /// prefix shortcut must not fire on it.
    #[test]
    fn a_short_exact_pin_is_still_rewritten() {
        assert_eq!(rewrite("=1", "1.0.229"), Rewrite::To("=1.0.229".into()));
    }

    /// Shapes this does not model are refused, never guessed at.
    #[test]
    fn an_unmodelled_requirement_is_refused_rather_than_rewritten() {
        assert!(matches!(
            rewrite(">=1.2, <2", "1.5.0"),
            Rewrite::Unsupported(_)
        ));
        assert!(matches!(rewrite("~1.2", "1.3.0"), Rewrite::Unsupported(_)));
        assert!(matches!(rewrite("1.*", "1.3.0"), Rewrite::Unsupported(_)));
        assert!(matches!(rewrite(">1.2", "1.3.0"), Rewrite::Unsupported(_)));
        assert!(matches!(rewrite("", "1.3.0"), Rewrite::Unsupported(_)));
    }

    /// A broad requirement in the manifest reports "nothing written"
    /// rather than failing or silently claiming an edit.
    #[test]
    fn an_already_covered_entry_reports_that_nothing_was_written() {
        let dir = tempfile::tempdir().unwrap();
        write(
            dir.path(),
            "Cargo.toml",
            "[package]\n\
             name = \"p\"\n\
             version = \"0.1.0\"\n\
             \n\
             [dependencies]\n\
             # WHY: broad on purpose.\n\
             serde = \"1\"\n",
        );

        let e = apply(dir.path(), "serde", "1.0.229").unwrap();
        assert!(!e.written);
        assert_eq!(e.before, "1");
        assert_eq!(e.after, "1");

        let text = std::fs::read_to_string(dir.path().join("Cargo.toml")).unwrap();
        assert!(
            text.contains("serde = \"1\""),
            "the file is untouched: {text}"
        );
    }

    /// A path dependency has no published version and is refused before
    /// anything is written.
    #[test]
    fn a_path_dependency_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        write(
            dir.path(),
            "Cargo.toml",
            "[package]\n\
             name = \"p\"\n\
             version = \"0.1.0\"\n\
             \n\
             [dependencies]\n\
             sibling = { path = \"../sibling\" }\n",
        );
        let e = apply(dir.path(), "sibling", "1.0.0").unwrap_err();
        assert!(e.contains("path dependency"), "{e}");
    }

    /// A crate nothing declares is refused rather than added.
    ///
    /// This is the guard against measured failure 2's other half: an
    /// apply must never CREATE a dependency.
    #[test]
    fn a_crate_no_manifest_declares_is_refused_rather_than_added() {
        let dir = tempfile::tempdir().unwrap();
        non_virtual_workspace(dir.path());
        let e = apply(dir.path(), "not-here", "1.0.0").unwrap_err();
        assert!(e.contains("not declared"), "{e}");

        let root = std::fs::read_to_string(dir.path().join("Cargo.toml")).unwrap();
        assert!(!root.contains("not-here"), "nothing was added: {root}");
    }

    /// A renamed crate is found by its crates.io name and edited under
    /// its LOCAL key, because that is what the file says.
    #[test]
    fn a_renamed_crate_is_edited_under_its_local_key() {
        let dir = tempfile::tempdir().unwrap();
        write(
            dir.path(),
            "Cargo.toml",
            "[package]\n\
             name = \"p\"\n\
             version = \"0.1.0\"\n\
             \n\
             [dependencies]\n\
             # WHY: aliased to avoid a clash.\n\
             local-name = { package = \"thiserror\", version = \"2.0.17\" }\n",
        );

        let e = apply(dir.path(), "thiserror", "2.0.18").unwrap();
        assert_eq!(e.after, "2.0.18");

        let text = std::fs::read_to_string(dir.path().join("Cargo.toml")).unwrap();
        assert!(
            text.contains("local-name = { package = \"thiserror\", version = \"2.0.18\" }"),
            "{text}"
        );
    }

    /// A real apply against a COPY of this repository's own manifests.
    ///
    /// `#[ignore]`d like `a_yarn_berry_project_reports_updates`: it
    /// needs the repository on disk at a known path. Run with
    /// `cargo test -- --ignored real_manifests` from `src-tauri`.
    ///
    /// The point is the one thing unit fixtures cannot prove: that this
    /// survives contact with `src-tauri/Cargo.toml`, which carries the
    /// densest why-comments in the repository -- several of them
    /// multi-line paragraphs above a single dependency.
    #[test]
    #[ignore = "needs this repository on disk; run explicitly"]
    fn real_manifests_keep_their_comments() {
        let repo = Path::new(env!("CARGO_MANIFEST_DIR"));
        let dir = tempfile::tempdir().unwrap();
        let project = dir.path().join("src-tauri");
        std::fs::create_dir_all(&project).unwrap();
        std::fs::copy(repo.join("Cargo.toml"), project.join("Cargo.toml")).unwrap();

        let before = std::fs::read_to_string(project.join("Cargo.toml")).unwrap();
        // A crate this manifest really declares, with a comment above.
        let e = apply(&project, "tokio", "1.99.0").unwrap();
        let after = std::fs::read_to_string(project.join("Cargo.toml")).unwrap();

        // EVERY comment line survives. Not a sample -- all of them.
        let comments = |t: &str| -> Vec<String> {
            t.lines()
                .map(str::trim)
                .filter(|l| l.starts_with('#'))
                .map(str::to_string)
                .collect()
        };
        assert_eq!(
            comments(&before),
            comments(&after),
            "every comment in the real manifest survived"
        );

        // Exactly ONE line differs, and it is the one asked for.
        let changed: Vec<(&str, &str)> = before
            .lines()
            .zip(after.lines())
            .filter(|(a, b)| a != b)
            .collect();
        assert_eq!(changed.len(), 1, "one line changed: {changed:?}");
        assert!(changed[0].1.contains("1.99.0"), "{:?}", changed[0]);
        assert_eq!(before.lines().count(), after.lines().count());
        assert!(e.written);
    }

    /// The quoted middle component of a target table must survive
    /// splitting, or nothing is ever found under a target.
    #[test]
    fn a_quoted_table_component_is_not_split_on_its_dots() {
        assert_eq!(
            table_parts("target.'cfg(target_os = \"ios\")'.dependencies"),
            vec![
                "target".to_string(),
                "cfg(target_os = \"ios\")".to_string(),
                "dependencies".to_string()
            ]
        );
        assert_eq!(
            table_parts("dependencies"),
            vec!["dependencies".to_string()]
        );
        assert_eq!(
            table_parts("workspace.dependencies"),
            vec!["workspace".to_string(), "dependencies".to_string()]
        );
    }
}
