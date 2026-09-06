//! Latest versions for the ecosystems with no local command.
//!
//! Terraform providers and Swift packages are the two whose "what is the
//! newest version" answer lives in an HTTP registry rather than in a
//! tool on the machine. Every other ecosystem here spawns a subprocess
//! and parses its stdout; these two cannot, because there is nothing to
//! spawn -- `terraform providers` reports CONSTRAINTS and does not diff,
//! and nothing at all reports outdated Xcode-managed Swift packages.
//!
//! That makes this the only part of `packages` that is async, and it is
//! why the check runs in two phases: a synchronous pass reports the
//! pinned versions immediately, and this fills in the latest afterwards.
//! A Terraform repository renders its providers at once rather than
//! blocking on the network.
//!
//! Nothing here writes to a file or runs a command.

use std::time::Duration;

/// Bounded, because a hung registry must not hold a check open. Short:
/// these are single small JSON responses, not downloads.
const TIMEOUT: Duration = Duration::from_secs(10);

/// The newest release version among a set of candidates.
///
/// SORTED SEMANTICALLY, never "the last one in the response".
///
/// The Terraform registry returns versions in no meaningful order --
/// measured on `hashicorp/archive`, the final three are
/// `2.7.1, 2.8.0, 2.4.1`. Taking the last element there reports a
/// DOWNGRADE as an update, which is the sort of confidently-wrong number
/// this module exists to avoid.
///
/// Prereleases are excluded: `1.0.0-rc.1` is not an upgrade from
/// `0.9.0` that anyone asked for, and the ecosystems here pin releases.
pub fn newest(versions: impl IntoIterator<Item = String>) -> Option<String> {
    versions
        .into_iter()
        .filter(|v| !is_prerelease(v))
        .filter_map(|v| super::version::numeric_parts(&v).map(|p| (p, v)))
        .max_by(|(a, _), (b, _)| a.cmp(b))
        .map(|(_, v)| v)
}

/// Whether a version string carries a prerelease marker.
///
/// Deliberately textual rather than a semver parse: the two registries
/// here disagree about form (`1.0.0-rc1`, `1.0.0-beta.2`, `2.0.0rc1`),
/// and a parser strict enough to reject one would reject real releases
/// from the other.
fn is_prerelease(v: &str) -> bool {
    let lower = v.to_ascii_lowercase();
    ["-rc", "-alpha", "-beta", "-pre", "-dev", "-snapshot"]
        .iter()
        .any(|m| lower.contains(m))
}

/// Fill in the latest version for every row that needs the network.
///
/// The SECOND phase. `check` returns Terraform and Swift rows with
/// `latest` equal to `current` and `bump` unknown, because it cannot
/// know better without asking. This asks.
///
/// Failures are per-ROW and silent by design: a provider whose registry
/// lookup fails keeps `latest == current` and `Bump::Unknown`, which
/// renders as "cannot compare" rather than as "up to date". One
/// unreachable host must not turn a whole repository's report into a
/// false all-clear -- the inversion this module exists to avoid.
pub async fn enrich(reports: &mut [super::model::ProjectReport]) {
    install_crypto_provider();
    let client = match reqwest::Client::builder().timeout(TIMEOUT).build() {
        Ok(c) => c,
        // No client, no enrichment. Rows keep Unknown, which is honest.
        Err(_) => return,
    };

    // De-duplicated: a real Terraform repo pins the same seven providers
    // across a dozen modules, which would otherwise be a dozen identical
    // requests.
    let mut wanted: Vec<(super::model::Ecosystem, String)> = Vec::new();
    for p in reports.iter() {
        for r in &p.reports {
            for o in &r.outdated {
                let key = lookup_key(o);
                if needs_lookup(o.ecosystem) && !wanted.iter().any(|(_, n)| n == &key) {
                    wanted.push((o.ecosystem, key));
                }
            }
        }
    }
    if wanted.is_empty() {
        return;
    }

    let mut found: std::collections::BTreeMap<String, String> = Default::default();
    for (eco, name) in wanted {
        if let Some(v) = latest_for(&client, eco, &name).await {
            found.insert(name, v);
        }
    }

    apply_found(reports, &found);
}

/// Write the looked-up versions onto the rows they belong to.
///
/// Split out of `enrich` so the decision below can be tested without a
/// network call -- `enrich` itself is entirely I/O.
///
/// A registry answer OLDER than the pin is not written. `newest` sorts
/// semantically, so what comes back is genuinely the newest release the
/// registry has; it can still be older than what the repository pins,
/// when a release has been yanked or the dependency came from somewhere
/// else. Writing it would offer a downgrade -- the same bug the parsers
/// filter, arriving by the other route.
///
/// The row is KEPT rather than dropped, which is the opposite of what
/// `run::keep` does and deliberately so. These rows come from a lock
/// file and represent a pinned dependency, not a claimed update, so
/// removing one would make a provider disappear from the list. Left
/// untouched, it stays at `latest == current` with `Bump::Unknown` --
/// exactly how a failed lookup already renders, which the UI shows as
/// "cannot compare" and the wizard refuses to select.
fn apply_found(
    reports: &mut [super::model::ProjectReport],
    found: &std::collections::BTreeMap<String, String>,
) {
    for p in reports.iter_mut() {
        for r in &mut p.reports {
            for o in &mut r.outdated {
                let Some(latest) = found.get(&lookup_key(o)) else {
                    continue;
                };
                if super::version::is_newer(&o.current, latest) == Some(false) {
                    log::warn!(
                        "the {} registry's newest release for {} is {latest}, older than the \
                         pinned {}; left uncompared rather than offered as an update",
                        o.ecosystem.program(),
                        o.name,
                        o.current
                    );
                    continue;
                }
                o.bump = super::version::bump(&o.current, latest);
                o.latest = latest.clone();
            }
        }
    }
}

/// The newest release TAG for a Swift package on GitHub.
///
/// Swift package versions are git tags, so this is a tag listing. Only
/// GitHub: anything else reports nothing rather than being guessed at,
/// and `latest` stays equal to `current` with `Bump::Unknown`, which
/// renders as "cannot check".
///
/// The public REST endpoint rather than the authenticated client. These
/// are public repositories by definition -- a Swift package resolved
/// from a URL is fetchable -- and routing this through octocrab would
/// spend the user's rate limit on someone else's dependency list.
async fn swift_latest(client: &reqwest::Client, location: &str) -> Option<String> {
    let (owner, repo) = super::swift::github_repo(location)?;
    let url = format!("https://api.github.com/repos/{owner}/{repo}/tags?per_page=100");
    let body: serde_json::Value = client
        .get(&url)
        // GitHub rejects an absent User-Agent outright.
        .header("User-Agent", "headstate")
        .send()
        .await
        .ok()?
        .json()
        .await
        .ok()?;
    let tags = body.as_array()?;
    newest(
        tags.iter()
            .filter_map(|t| t.get("name")?.as_str().map(str::to_string)),
    )
}

/// Install rustls' crypto provider, once.
///
/// `reqwest` is declared with `json` only -- deliberately, because its
/// `rustls` feature drags in quinn (QUIC) and a crypto stack, 125 lock
/// entries, despite the crate already being present. Without a TLS
/// feature reqwest builds a client fine and then PANICS on first use
/// with "No rustls crypto provider is configured".
///
/// `ring` rather than `aws-lc-rs`: octocrab is built with `rustls-ring`,
/// so this is the provider already compiled in and adds nothing.
///
/// `OnceLock` because installing twice returns an error, and racing
/// tasks would otherwise both try.
fn install_crypto_provider() {
    static ONCE: std::sync::OnceLock<()> = std::sync::OnceLock::new();
    ONCE.get_or_init(|| {
        // Err means one is ALREADY installed, which is the desired end
        // state -- octocrab may have got there first.
        let _ = rustls::crypto::ring::default_provider().install_default();
    });
}

/// What to ask the registry about, which is not always the display name.
///
/// Terraform's `name` IS the full provider address, so it is the key.
/// Swift's name was shortened for the list -- `sqlcipher/SQLCipher.swift`
/// rather than the `.git` URL -- so the source URL is carried at the
/// front of `manifest` and recovered here.
fn lookup_key(o: &super::model::Outdated) -> String {
    match o.ecosystem {
        super::model::Ecosystem::Swift => o
            .manifest
            .split_once(" <- ")
            .map_or_else(|| o.name.clone(), |(url, _)| url.to_string()),
        _ => o.name.clone(),
    }
}

/// Whether this ecosystem's latest version comes from a registry.
fn needs_lookup(eco: super::model::Ecosystem) -> bool {
    matches!(
        eco,
        super::model::Ecosystem::Terraform | super::model::Ecosystem::Swift
    )
}

/// The newest published version for one dependency.
async fn latest_for(
    client: &reqwest::Client,
    eco: super::model::Ecosystem,
    name: &str,
) -> Option<String> {
    match eco {
        super::model::Ecosystem::Terraform => terraform_latest(client, name).await,
        super::model::Ecosystem::Swift => swift_latest(client, name).await,
        _ => None,
    }
}

/// Terraform's registry, from a full provider address.
///
/// `registry.terraform.io/hashicorp/archive` becomes
/// `https://registry.terraform.io/v1/providers/hashicorp/archive/versions`.
/// The host is taken from the address rather than assumed, so a private
/// registry is not silently queried against HashiCorp's.
async fn terraform_latest(client: &reqwest::Client, source: &str) -> Option<String> {
    let mut parts = source.splitn(3, '/');
    let host = parts.next()?;
    let namespace = parts.next()?;
    let name = parts.next()?;
    if host.is_empty() || namespace.is_empty() || name.is_empty() {
        return None;
    }
    let url = format!("https://{host}/v1/providers/{namespace}/{name}/versions");
    let body: serde_json::Value = client.get(&url).send().await.ok()?.json().await.ok()?;
    let versions = body.get("versions")?.as_array()?;
    newest(
        versions
            .iter()
            .filter_map(|v| v.get("version")?.as_str().map(str::to_string)),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The REAL tail of `hashicorp/archive`'s version list, in the order
    /// the registry returned it. Taking the last element reports 2.4.1
    /// as latest -- a downgrade from 2.8.0.
    #[test]
    fn picks_the_newest_from_an_unsorted_response() {
        let versions = ["2.7.1", "2.8.0", "2.4.1"].map(String::from);
        assert_eq!(newest(versions), Some("2.8.0".into()));
    }

    /// Explicitly NOT last-wins, which is the bug this guards.
    #[test]
    fn the_last_entry_is_not_assumed_newest() {
        let versions = ["9.9.9", "1.0.0"].map(String::from);
        assert_eq!(newest(versions), Some("9.9.9".into()));
    }

    /// A prerelease is not an upgrade anyone asked for.
    #[test]
    fn prereleases_are_excluded() {
        let versions = ["1.0.0", "2.0.0-rc.1", "2.0.0-beta.2"].map(String::from);
        assert_eq!(newest(versions), Some("1.0.0".into()));
    }

    /// Numeric ordering, not lexical: "10" sorts after "9".
    #[test]
    fn compares_numerically_not_as_text() {
        let versions = ["1.9.0", "1.10.0"].map(String::from);
        assert_eq!(newest(versions), Some("1.10.0".into()));
    }

    /// Swift tags are commonly `v`-prefixed.
    #[test]
    fn a_v_prefix_does_not_break_the_comparison() {
        let versions = ["v1.2.0", "v1.10.0"].map(String::from);
        assert_eq!(newest(versions), Some("v1.10.0".into()));
    }

    /// Nothing comparable means NO ANSWER, never a guess -- the same
    /// rule `Bump::Unknown` follows.
    #[test]
    fn uncomparable_input_yields_none() {
        assert_eq!(newest(Vec::<String>::new()), None);
        assert_eq!(newest(["main".to_string(), "trunk".to_string()]), None);
    }

    /// All-prerelease is "no release yet", not "the newest prerelease".
    #[test]
    fn only_prereleases_yields_none() {
        assert_eq!(newest(["1.0.0-rc.1".to_string()]), None);
    }

    /// A pin NEWER than anything published is not an update.
    ///
    /// `newest` sorts semantically, so the answer it returns is the
    /// newest release the registry has -- but that can still be older
    /// than what the repository pins: a yanked release, or a version
    /// installed from somewhere other than this registry. Writing it
    /// into `latest` would offer a downgrade, which is the same bug the
    /// parsers filter, arriving by the other route.
    ///
    /// The Terraform and Swift rows are NOT dropped for it, unlike a
    /// parser row. They are built from a lock file and represent a
    /// pinned dependency rather than a claimed update, so removing one
    /// would make a provider vanish from the list entirely. Left at
    /// `latest == current` with `Bump::Unknown`, they render exactly as
    /// a failed lookup does: "cannot compare", never "up to date".
    #[test]
    fn a_registry_answer_older_than_the_pin_is_not_applied() {
        use super::super::model::{Bump, Ecosystem, EcosystemReport, Outdated, ProjectReport};
        let row = |current: &str| Outdated {
            name: "registry.terraform.io/hashicorp/archive".into(),
            current: current.into(),
            latest: current.into(),
            bump: Bump::Unknown,
            ecosystem: Ecosystem::Terraform,
            manifest: ".terraform.lock.hcl".into(),
        };
        let mut reports = vec![ProjectReport {
            path: "/tmp/example".into(),
            label: String::new(),
            reports: vec![EcosystemReport {
                ecosystem: Ecosystem::Terraform,
                // Pinned ahead of the registry, and pinned behind it.
                outdated: vec![row("9.9.9"), row("2.0.0")],
                error: None,
            }],
        }];

        apply_found(
            &mut reports,
            &[(
                "registry.terraform.io/hashicorp/archive".to_string(),
                "2.8.0".to_string(),
            )]
            .into_iter()
            .collect(),
        );

        let rows = &reports[0].reports[0].outdated;
        assert_eq!(rows.len(), 2, "neither row is dropped");
        assert_eq!(
            rows[0].latest, "9.9.9",
            "the backwards answer is not written"
        );
        assert_eq!(rows[0].bump, Bump::Unknown, "and it reads as uncomparable");
        assert_eq!(rows[1].latest, "2.8.0", "the real update still lands");
        assert_eq!(rows[1].bump, Bump::Minor);
    }

    /// A Swift package pinned to a REVISION keeps its row whichever way
    /// the comparison happens to fall.
    ///
    /// `swift::pinned` reports a branch or bare-commit pin with its
    /// seven-character revision and `Bump::Unknown`, on purpose: there
    /// is no version to compare, and calling it current would be a lie.
    /// A revision beginning with a digit (`9f2a1c4`) is the one that
    /// `numeric_parts` mistakes for a version, so it takes the
    /// suppression path above -- and lands in the right place anyway,
    /// because that path leaves the row exactly as `swift::pinned`
    /// built it. Neither revision is dropped, and neither is claimed to
    /// be an update.
    #[test]
    fn a_swift_revision_pin_survives_enrichment_either_way() {
        use super::super::model::{Bump, Ecosystem, EcosystemReport, Outdated, ProjectReport};
        let url = "https://github.com/octocat/hello-world.git";
        let row = |revision: &str| Outdated {
            name: "octocat/hello-world".into(),
            current: revision.into(),
            latest: revision.into(),
            bump: Bump::Unknown,
            ecosystem: Ecosystem::Swift,
            manifest: format!("{url} <- Package.resolved"),
        };
        let mut reports = vec![ProjectReport {
            path: "/tmp/example".into(),
            label: String::new(),
            // One revision that reads as a big number, one that does
            // not read as a version at all.
            reports: vec![EcosystemReport {
                ecosystem: Ecosystem::Swift,
                outdated: vec![row("9f2a1c4"), row("f2a1c4d")],
                error: None,
            }],
        }];

        apply_found(
            &mut reports,
            &[(url.to_string(), "2.0.0".to_string())]
                .into_iter()
                .collect(),
        );

        let rows = &reports[0].reports[0].outdated;
        assert_eq!(rows.len(), 2, "a revision pin is reported, never dropped");
        assert_eq!(rows[0].latest, "9f2a1c4", "no update claimed against it");
        assert_eq!(rows[0].bump, Bump::Unknown);
        // The other revision is not comparable at all, so enrichment
        // writes the tag -- and `bump` still answers Unknown, which is
        // what the row must show.
        assert_eq!(rows[1].bump, Bump::Unknown, "still uncomparable");
    }
}

/// Against the LIVE registry. Ignored: needs the network.
///
/// `HEADSTATE_TF_REPO=/path cargo test -- --ignored live --nocapture`
#[cfg(test)]
mod live {
    use super::*;

    #[tokio::test]
    #[ignore = "needs network access and a real Terraform repository"]
    async fn enriches_a_real_repository() {
        let Ok(repo) = std::env::var("HEADSTATE_TF_REPO") else {
            eprintln!("set HEADSTATE_TF_REPO");
            return;
        };
        let mut reports = super::super::run::check_repo(std::path::Path::new(&repo));
        let before: Vec<String> = reports
            .iter()
            .flat_map(|p| &p.reports)
            .flat_map(|r| &r.outdated)
            .map(|o| format!("{} {} -> {}", o.name, o.current, o.latest))
            .take(3)
            .collect();
        eprintln!("BEFORE: {before:#?}");

        enrich(&mut reports).await;

        let rows: Vec<_> = reports
            .iter()
            .flat_map(|p| &p.reports)
            .flat_map(|r| &r.outdated)
            .collect();
        eprintln!("rows: {}", rows.len());
        for o in rows.iter().take(4) {
            eprintln!("  {} {} -> {} ({:?})", o.name, o.current, o.latest, o.bump);
        }
        let moved = rows.iter().filter(|o| o.latest != o.current).count();
        eprintln!("rows with a NEWER version available: {moved}");
        assert!(!rows.is_empty(), "the repo must yield providers");
    }
}

#[cfg(test)]
mod live_swift {
    use super::*;

    #[tokio::test]
    #[ignore = "needs network access and a real Swift project"]
    async fn enriches_a_real_swift_project() {
        let Ok(repo) = std::env::var("HEADSTATE_SWIFT_REPO") else {
            eprintln!("set HEADSTATE_SWIFT_REPO");
            return;
        };
        let mut reports = super::super::run::check_repo(std::path::Path::new(&repo));
        enrich(&mut reports).await;
        let rows: Vec<_> = reports
            .iter()
            .flat_map(|p| &p.reports)
            .filter(|r| r.ecosystem == super::super::model::Ecosystem::Swift)
            .flat_map(|r| &r.outdated)
            .collect();
        eprintln!("swift rows: {}", rows.len());
        for o in rows.iter().take(5) {
            eprintln!("  {} {} -> {} ({:?})", o.name, o.current, o.latest, o.bump);
        }
    }
}
