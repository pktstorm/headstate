use super::model::Ecosystem;
use std::path::Path;

/// One project within a repository, and what it uses.
///
/// A repository is not one project. Measured on a real machine, one repo
/// held `frontend/package.json`, `backend/pyproject.toml`, and a third
/// service beside them -- and the root-only check found NONE of them,
/// rendering the page as though there were nothing to update.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Project {
    /// Absolute path to the directory holding the manifest.
    pub path: String,
    /// Relative to the repository root, for display. Empty at the root.
    pub label: String,
    pub ecosystems: Vec<Ecosystem>,
}

/// Every project in a repository, root included.
///
/// Bounded in depth and skipping the directories that hold other
/// people's manifests: a `node_modules` tree contains thousands of
/// `package.json` files, none of them this repository's.
pub fn projects(repo: &Path) -> Vec<Project> {
    const MAX_DEPTH: usize = 3;
    const SKIP: &[&str] = &[
        ".git",
        "node_modules",
        "target",
        ".terraform",
        ".venv",
        "venv",
        "dist",
        "build",
        "bin",
        "obj",
        ".worktrees",
    ];

    let mut out: Vec<Project> = Vec::new();

    // Breadth-first, so a parent is always recorded before its children
    // and `claimed_by_ancestor` can see it.
    let mut queue = std::collections::VecDeque::from([(repo.to_path_buf(), 0usize)]);

    while let Some((dir, depth)) = queue.pop_front() {
        // Only the ecosystems no ANCESTOR project already covers.
        //
        // A project's subdirectories are not separate projects for the
        // toolchain it OWNS: a yarn workspace member is the workspace's
        // business, and the tool reports it from the root. That was
        // implemented by ending the descent entirely, which is a claim
        // about the directory rather than about one toolchain -- and it
        // is wrong for any polyglot layout.
        //
        // A Tauri repository is exactly that: `package.json` at the root
        // with `src-tauri/Cargo.toml` beneath it. Stopping at the root
        // meant the Rust half was never looked at. Measured on THIS
        // repository -- four Cargo manifests, zero rows.
        //
        // So the descent continues and the filter is per ecosystem: a
        // yarn member under a yarn root still adds nothing, a crate
        // under a yarn root becomes its own project.
        let claimed = claimed_by_ancestor(&out, &dir);
        let ecos: Vec<Ecosystem> = ecosystems(&dir)
            .into_iter()
            .filter(|e| !claimed.contains(e))
            .collect();

        if !ecos.is_empty() {
            let label = dir
                .strip_prefix(repo)
                .map(|p| p.to_string_lossy().to_string())
                .unwrap_or_default();
            out.push(Project {
                path: dir.to_string_lossy().to_string(),
                label,
                ecosystems: ecos,
            });
        }

        if depth >= MAX_DEPTH {
            continue;
        }
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        // Sorted, so the traversal does not depend on directory order.
        let mut children: Vec<std::path::PathBuf> = Vec::new();
        for e in entries.flatten() {
            let Ok(meta) = e.metadata() else { continue };
            if !meta.is_dir() {
                continue;
            }
            let name = e.file_name().to_string_lossy().to_string();
            if SKIP.contains(&name.as_str()) || name.starts_with('.') {
                continue;
            }
            children.push(e.path());
        }
        children.sort();
        for c in children {
            queue.push_back((c, depth + 1));
        }
    }

    // Root first, then alphabetical -- a stable order that does not
    // reshuffle between scans.
    out.sort_by(|a, b| {
        a.label
            .len()
            .cmp(&b.label.len())
            .then(a.label.cmp(&b.label))
    });
    out
}

/// The ecosystems an already-recorded project ABOVE this directory
/// covers.
///
/// A path-prefix test, not a string one: `frontend-tools` is not inside
/// `frontend`, and comparing the strings would say it was.
fn claimed_by_ancestor(found: &[Project], dir: &Path) -> Vec<Ecosystem> {
    found
        .iter()
        .filter(|p| {
            let parent = Path::new(&p.path);
            parent != dir && dir.starts_with(parent)
        })
        .flat_map(|p| p.ecosystems.iter().copied())
        .collect()
}

/// Which ecosystems a repository actually uses.
///
/// Detected from manifests on disk rather than guessed, and a repo can
/// legitimately return several: a Python service with a web frontend has
/// both, and reporting only the first would hide half its dependencies.
pub fn ecosystems(repo: &Path) -> Vec<Ecosystem> {
    let mut out = Vec::new();

    if repo.join("package.json").is_file() {
        // Yarn and npm share `package.json`, so the LOCKFILE decides.
        // Asking the wrong tool produces a confident empty list rather
        // than an error, which is the failure mode this whole module is
        // built to avoid.
        if repo.join("yarn.lock").is_file() {
            out.push(Ecosystem::Yarn);
        } else {
            out.push(Ecosystem::Npm);
        }
    }

    if repo.join("pyproject.toml").is_file() {
        // Same problem again: both tools use `pyproject.toml`. `uv.lock`
        // is uv's; `poetry.lock` is Poetry's. With neither, the
        // `[tool.*]` table is the tiebreak.
        if repo.join("uv.lock").is_file() {
            out.push(Ecosystem::Uv);
        } else if repo.join("poetry.lock").is_file() {
            out.push(Ecosystem::Poetry);
        } else if let Ok(text) = std::fs::read_to_string(repo.join("pyproject.toml")) {
            if text.contains("[tool.uv") {
                out.push(Ecosystem::Uv);
            } else if text.contains("[tool.poetry") {
                out.push(Ecosystem::Poetry);
            }
            // Neither table and no lockfile: a pyproject.toml that
            // belongs to some third tool. Reporting nothing beats
            // running the wrong one.
        }
    }

    if has_project_file(repo) {
        out.push(Ecosystem::Dotnet);
    }

    if declares_pods(repo) {
        out.push(Ecosystem::Cocoapods);
    }

    // Terraform, if any lock file exists ANYWHERE in the repo rather
    // than only in this directory. A Terraform repository is commonly
    // many rooted modules -- `modules/*/`, `environments/*/` -- each
    // with its own lock, and matching only the project directory finds
    // nothing on a real one.
    if !crate::packages::terraform::pinned(repo).is_empty() {
        out.push(Ecosystem::Terraform);
    }

    // Swift: a package of its own, or dependencies Xcode manages.
    //
    // The Xcode case is the one that matters for iOS repositories, and
    // its `Package.resolved` is buried inside the project bundle rather
    // than sitting at the root -- which is why a root-only check found
    // nothing on a real iOS repo that plainly uses SPM.
    if repo.join("Package.swift").is_file() || has_xcode_spm(repo) {
        out.push(Ecosystem::Swift);
    }

    // Cargo, from the manifest alone.
    //
    // No lockfile tiebreak is needed -- unlike `package.json` and
    // `pyproject.toml`, nothing else owns `Cargo.toml`. A workspace root
    // and a standalone crate both match, which is correct: a workspace
    // root is ONE project, and `projects` already stops descending once
    // a directory has an ecosystem, so its members never become separate
    // rows with their own update commands. `packages::cargo` reads the
    // members from the root and reports them under it.
    if repo.join("Cargo.toml").is_file() {
        out.push(Ecosystem::Cargo);
    }

    out
}

/// Whether Xcode manages Swift packages here.
///
/// `Package.resolved` lives under the `.xcodeproj` or `.xcworkspace`
/// bundle. Its presence is what distinguishes a project WITH
/// dependencies from one without.
fn has_xcode_spm(dir: &Path) -> bool {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return false;
    };
    entries.flatten().any(|e| {
        let p = e.path();
        let is_bundle = p
            .extension()
            .and_then(|x| x.to_str())
            .is_some_and(|x| x == "xcodeproj" || x == "xcworkspace");
        is_bundle
            && p.join("project.xcworkspace/xcshareddata/swiftpm/Package.resolved")
                .is_file()
    })
}

/// Whether this directory is really a CocoaPods project.
///
/// A bare `Podfile.is_file()` was not enough. `tauri ios init` writes a
/// Podfile with two targets and no pods in either, and this repo carries
/// one at `src-mobile/gen/apple/`. That stub made every scan run
/// `pod outdated`, which refused for want of a `Podfile.lock` -- so the
/// Packages page showed a standing CocoaPods warning for an ecosystem
/// the project does not use. The warning itself was right (#567 made
/// refusals visible rather than silently reporting "no updates"); it is
/// the detection underneath that was wrong.
///
/// A `Podfile.lock` is enough on its own: pods existed at some point,
/// and whatever the Podfile says now, there is a resolved set worth
/// scanning. Otherwise a `pod` declaration is what makes the ecosystem
/// real. A Podfile with dependencies and no lockfile still warns --
/// that is a genuine finding and the case this must not swallow.
fn declares_pods(dir: &Path) -> bool {
    if !dir.join("Podfile").is_file() {
        return false;
    }
    if dir.join("Podfile.lock").is_file() {
        return true;
    }
    let Ok(text) = std::fs::read_to_string(dir.join("Podfile")) else {
        // Unreadable rather than absent: something is there, and
        // reporting nothing would hide a real project behind a
        // permissions error. Scanning says so out loud instead.
        return true;
    };
    text.lines().any(|line| {
        let line = line.trim_start();
        // `pod` as the statement, not as a prefix: `pod 'Alamofire'`
        // counts, `pod_target_xcconfig` does not. The stub's own
        // "# Pods for ..." comment is excluded by the same rule.
        line.strip_prefix("pod")
            .is_some_and(|rest| rest.starts_with([' ', '\t', '\'', '"', '(']))
    })
}

/// Whether a .NET project or solution file sits here.
fn has_project_file(dir: &Path) -> bool {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return false;
    };
    entries.flatten().any(|e| {
        e.path()
            .extension()
            .and_then(|x| x.to_str())
            .is_some_and(|x| {
                ["csproj", "fsproj", "vbproj", "sln"]
                    .iter()
                    .any(|w| x.eq_ignore_ascii_case(w))
            })
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn repo() -> tempfile::TempDir {
        tempfile::TempDir::new().unwrap()
    }

    #[test]
    fn a_repo_with_nothing_uses_nothing() {
        assert!(ecosystems(repo().path()).is_empty());
    }

    /// npm and yarn share `package.json`, so the lockfile decides.
    /// Asking the wrong tool returns a confident EMPTY list rather than
    /// an error, which is the exact failure this module exists to avoid.
    #[test]
    fn the_lockfile_decides_between_npm_and_yarn() {
        let t = repo();
        fs::write(t.path().join("package.json"), "{}").unwrap();
        assert_eq!(ecosystems(t.path()), vec![Ecosystem::Npm]);

        fs::write(t.path().join("yarn.lock"), "").unwrap();
        assert_eq!(ecosystems(t.path()), vec![Ecosystem::Yarn]);
    }

    /// Same problem for Python: both tools own `pyproject.toml`.
    #[test]
    fn the_lockfile_decides_between_poetry_and_uv() {
        let t = repo();
        fs::write(t.path().join("pyproject.toml"), "[project]").unwrap();
        fs::write(t.path().join("poetry.lock"), "").unwrap();
        assert_eq!(ecosystems(t.path()), vec![Ecosystem::Poetry]);

        let t2 = repo();
        fs::write(t2.path().join("pyproject.toml"), "[project]").unwrap();
        fs::write(t2.path().join("uv.lock"), "").unwrap();
        assert_eq!(ecosystems(t2.path()), vec![Ecosystem::Uv]);
    }

    /// With no lockfile the `[tool.*]` table is the tiebreak.
    #[test]
    fn the_tool_table_breaks_the_tie_without_a_lockfile() {
        let t = repo();
        fs::write(t.path().join("pyproject.toml"), "[tool.poetry]\nname='x'").unwrap();
        assert_eq!(ecosystems(t.path()), vec![Ecosystem::Poetry]);
    }

    /// A `pyproject.toml` belonging to some third tool must report
    /// NOTHING rather than running one of ours against it.
    #[test]
    fn an_unrecognised_pyproject_reports_no_ecosystem() {
        let t = repo();
        fs::write(t.path().join("pyproject.toml"), "[build-system]").unwrap();
        assert!(ecosystems(t.path()).is_empty());
    }

    /// A repo can legitimately use several. Reporting only the first
    /// would hide half its dependencies.
    #[test]
    fn a_polyglot_repo_reports_every_ecosystem() {
        let t = repo();
        fs::write(t.path().join("package.json"), "{}").unwrap();
        fs::write(t.path().join("pyproject.toml"), "[tool.uv]").unwrap();
        fs::write(t.path().join("Api.csproj"), "<Project/>").unwrap();
        let found = ecosystems(t.path());
        assert_eq!(found.len(), 3, "{found:?}");
    }

    /// The reported case: a repository whose projects live one level
    /// down reported NOTHING, because only the root was checked.
    /// Measured on a real repo -- three projects, none found.
    #[test]
    fn nested_projects_are_found() {
        let t = repo();
        for (dir, file, body) in [
            ("frontend", "package.json", "{}"),
            ("backend", "pyproject.toml", "[tool.poetry]"),
            ("service", "pyproject.toml", "[tool.uv]"),
        ] {
            fs::create_dir(t.path().join(dir)).unwrap();
            fs::write(t.path().join(dir).join(file), body).unwrap();
        }

        let found = projects(t.path());
        assert_eq!(found.len(), 3, "{found:?}");
        let labels: Vec<&str> = found.iter().map(|p| p.label.as_str()).collect();
        assert!(labels.contains(&"frontend") && labels.contains(&"backend"));
    }

    /// A project's own subdirectories are not separate projects. A
    /// workspace member is the workspace's business, and the tool
    /// reports it from the root.
    #[test]
    fn a_nested_manifest_under_a_project_is_not_a_second_project() {
        let t = repo();
        fs::write(t.path().join("package.json"), "{}").unwrap();
        fs::create_dir_all(t.path().join("packages/inner")).unwrap();
        fs::write(t.path().join("packages/inner/package.json"), "{}").unwrap();

        let found = projects(t.path());
        assert_eq!(found.len(), 1, "the root subsumes its members: {found:?}");
        assert_eq!(found[0].label, "");
    }

    /// `node_modules` holds thousands of other people's manifests.
    #[test]
    fn dependency_directories_are_never_projects() {
        let t = repo();
        fs::create_dir_all(t.path().join("node_modules/left-pad")).unwrap();
        fs::write(t.path().join("node_modules/left-pad/package.json"), "{}").unwrap();
        assert!(projects(t.path()).is_empty());
    }

    #[test]
    fn cocoapods_is_detected_from_a_podfile_that_declares_pods() {
        let t = repo();
        // A `pod` line is what makes the ecosystem real. Note this test
        // used to write only `platform :ios`, which declares nothing --
        // it passed against the old bare file check and would have gone
        // on passing while the bug it now covers was live.
        fs::write(
            t.path().join("Podfile"),
            "target 'App' do\n  platform :ios, '14.0'\n  pod 'Alamofire', '~> 5.0'\nend\n",
        )
        .unwrap();
        assert_eq!(ecosystems(t.path()), vec![Ecosystem::Cocoapods]);
    }

    /// The stub `tauri ios init` writes -- THIS repo's own
    /// `src-mobile/gen/apple/Podfile`, read from disk rather than
    /// retyped, so the test cannot drift from the file that caused the
    /// bug. Both targets are empty, no lockfile has ever been written,
    /// and nothing in CI runs CocoaPods, yet the Packages page showed a
    /// standing warning for an ecosystem the project does not use.
    ///
    /// Reading the real file also covers the `post_install` block a
    /// hand-written fixture omitted -- a Ruby hook with no pods in it,
    /// and exactly the kind of thing a looser match would misread.
    #[test]
    fn the_generated_podfile_in_this_repo_is_not_a_cocoapods_project() {
        let generated =
            Path::new(env!("CARGO_MANIFEST_DIR")).join("../src-mobile/gen/apple/Podfile");
        // Skipped rather than failed when absent: `gen/apple` is
        // generated, and a checkout that has not run `tauri ios init`
        // is not broken.
        let Ok(text) = fs::read_to_string(&generated) else {
            return;
        };
        let t = repo();
        fs::write(t.path().join("Podfile"), text).unwrap();
        assert_eq!(ecosystems(t.path()), Vec::<Ecosystem>::new());
    }

    /// The case this must NOT swallow: real dependencies, no lockfile.
    /// That is a genuine finding, and hiding it was exactly the failure
    /// #567 fixed.
    #[test]
    fn pods_without_a_lockfile_still_warn() {
        let t = repo();
        fs::write(t.path().join("Podfile"), "  pod 'Alamofire'\n").unwrap();
        assert_eq!(ecosystems(t.path()), vec![Ecosystem::Cocoapods]);
    }

    /// A lockfile means pods existed, whatever the Podfile says now.
    #[test]
    fn an_empty_podfile_with_a_lockfile_is_still_scanned() {
        let t = repo();
        fs::write(t.path().join("Podfile"), "target 'App' do\nend\n").unwrap();
        fs::write(
            t.path().join("Podfile.lock"),
            "PODS:\n  - Alamofire (5.0)\n",
        )
        .unwrap();
        assert_eq!(ecosystems(t.path()), vec![Ecosystem::Cocoapods]);
    }

    /// `pod` as a statement, not as a prefix. `pod_target_xcconfig` is a
    /// configuration hook that appears in Podfiles with no dependencies.
    #[test]
    fn a_word_beginning_with_pod_is_not_a_pod_declaration() {
        let t = repo();
        fs::write(
            t.path().join("Podfile"),
            "target 'App' do\n  pod_target_xcconfig = {}\nend\n",
        )
        .unwrap();
        assert_eq!(ecosystems(t.path()), Vec::<Ecosystem>::new());
    }

    #[test]
    fn a_swift_package_is_detected() {
        let t = repo();
        fs::write(t.path().join("Package.swift"), "// swift-tools-version:5.9").unwrap();
        assert_eq!(ecosystems(t.path()), vec![Ecosystem::Swift]);
    }

    /// Xcode buries `Package.resolved` inside the project bundle, which
    /// is why a root-only check found nothing on a real iOS repository.
    #[test]
    fn xcode_managed_swift_packages_are_detected() {
        let t = repo();
        let deep = t
            .path()
            .join("App.xcodeproj/project.xcworkspace/xcshareddata/swiftpm");
        fs::create_dir_all(&deep).unwrap();
        fs::write(deep.join("Package.resolved"), "{}").unwrap();
        assert_eq!(ecosystems(t.path()), vec![Ecosystem::Swift]);
    }

    /// An `.xcodeproj` with NO resolved packages is a project without
    /// dependencies, not a Swift ecosystem to check.
    #[test]
    fn an_xcodeproj_without_resolved_packages_is_not_swift() {
        let t = repo();
        fs::create_dir(t.path().join("App.xcodeproj")).unwrap();
        assert!(ecosystems(t.path()).is_empty());
    }

    /// The Tauri shape, and the bug this found. A `package.json` at the
    /// root used to END the descent, so `src-tauri/Cargo.toml` beneath
    /// it was never even looked at. Measured on THIS repository: four
    /// Cargo manifests, zero rows.
    #[test]
    fn a_crate_under_a_javascript_root_is_still_found() {
        let t = repo();
        fs::write(t.path().join("package.json"), "{}").unwrap();
        fs::write(t.path().join("yarn.lock"), "").unwrap();
        fs::create_dir(t.path().join("src-tauri")).unwrap();
        fs::write(
            t.path().join("src-tauri/Cargo.toml"),
            "[package]\nname=\"app\"",
        )
        .unwrap();

        let found = projects(t.path());
        assert_eq!(found.len(), 2, "{found:?}");
        assert_eq!(found[0].ecosystems, vec![Ecosystem::Yarn]);
        assert_eq!(found[1].label, "src-tauri");
        assert_eq!(found[1].ecosystems, vec![Ecosystem::Cargo]);
    }

    /// The other half of that rule, and what keeps the fix narrow: a
    /// nested project in the SAME ecosystem still adds nothing, because
    /// it is the parent tool's business.
    #[test]
    fn a_nested_project_in_the_same_ecosystem_is_still_subsumed() {
        let t = repo();
        fs::write(t.path().join("Cargo.toml"), "[package]\nname=\"root\"").unwrap();
        fs::create_dir_all(t.path().join("crates/inner")).unwrap();
        fs::write(
            t.path().join("crates/inner/Cargo.toml"),
            "[package]\nname=\"inner\"",
        )
        .unwrap();

        let found = projects(t.path());
        assert_eq!(found.len(), 1, "the root subsumes it: {found:?}");
    }

    /// A sibling whose NAME starts with the parent's is not inside it.
    /// A string-prefix test would say it was, and silently drop its
    /// ecosystems as already covered.
    #[test]
    fn a_sibling_with_a_prefix_name_is_not_treated_as_nested() {
        let t = repo();
        for (dir, file, body) in [
            ("frontend", "package.json", "{}"),
            ("frontend-tools", "package.json", "{}"),
        ] {
            fs::create_dir(t.path().join(dir)).unwrap();
            fs::write(t.path().join(dir).join(file), body).unwrap();
        }
        let found = projects(t.path());
        assert_eq!(found.len(), 2, "{found:?}");
    }

    /// Nothing but Cargo owns `Cargo.toml`, so no lockfile tiebreak is
    /// needed the way npm/yarn and poetry/uv need one.
    #[test]
    fn cargo_is_detected_from_a_manifest() {
        let t = repo();
        fs::write(t.path().join("Cargo.toml"), "[package]\nname=\"x\"").unwrap();
        assert_eq!(ecosystems(t.path()), vec![Ecosystem::Cargo]);
    }

    /// A Cargo WORKSPACE ROOT is one project, and its members must not
    /// appear as separate rows -- each would carry its own update
    /// command against a manifest that inherits its versions from the
    /// root. This repo is the case: `src-mobile` is a workspace whose
    /// members are the two plugins under `plugins/`.
    #[test]
    fn a_cargo_workspace_root_is_one_project_not_one_per_member() {
        let t = repo();
        fs::write(
            t.path().join("Cargo.toml"),
            "[package]\nname=\"root\"\n[workspace]\nmembers = [\"plugins/one\", \"plugins/two\"]",
        )
        .unwrap();
        for m in ["plugins/one", "plugins/two"] {
            fs::create_dir_all(t.path().join(m)).unwrap();
            fs::write(t.path().join(m).join("Cargo.toml"), "[package]\nname=\"m\"").unwrap();
        }

        let found = projects(t.path());
        assert_eq!(found.len(), 1, "the root subsumes its members: {found:?}");
        assert_eq!(found[0].label, "");
        assert_eq!(found[0].ecosystems, vec![Ecosystem::Cargo]);
    }

    /// Two STANDALONE crates side by side are two projects, which is the
    /// other half of the repo's own shape: `src-tauri` is standalone and
    /// `src-mobile` is a workspace root.
    #[test]
    fn sibling_cargo_crates_are_separate_projects() {
        let t = repo();
        for (dir, body) in [
            ("desktop", "[package]\nname=\"desktop\""),
            (
                "mobile",
                "[package]\nname=\"mobile\"\n[workspace]\nmembers = []",
            ),
        ] {
            fs::create_dir(t.path().join(dir)).unwrap();
            fs::write(t.path().join(dir).join("Cargo.toml"), body).unwrap();
        }
        let found = projects(t.path());
        assert_eq!(found.len(), 2, "{found:?}");
    }

    #[test]
    fn dotnet_is_detected_from_any_project_or_solution_file() {
        for name in ["A.csproj", "B.fsproj", "C.vbproj", "D.sln", "E.CSPROJ"] {
            let t = repo();
            fs::write(t.path().join(name), "<Project/>").unwrap();
            assert_eq!(ecosystems(t.path()), vec![Ecosystem::Dotnet], "{name}");
        }
    }
}
