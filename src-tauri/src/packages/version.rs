use super::model::Bump;

/// How big a jump it is from `current` to `latest`.
///
/// Deliberately NOT a semver crate. Three of the five ecosystems here do
/// not use semver:
///
/// - .NET routinely ships four-part versions (`1.2.3.4`).
/// - PEP 440 allows epochs (`1!2.0`), local versions (`1.0+local`), and
///   suffixes like `1.0rc1` that a semver parser rejects.
/// - npm and Poetry are close to semver but still see `~`, `^`, and `v`
///   prefixes in the wild.
///
/// So this compares dotted numeric components and answers `Unknown` when
/// it genuinely cannot tell. That third answer is the point: a version
/// silently classified as major would hide from a "minors only" filter,
/// and one silently classified as minor would be offered as safe. Being
/// unable to compare is a fact the user should see, not one to guess past.
pub fn bump(current: &str, latest: &str) -> Bump {
    let (Some(a), Some(b)) = (numeric_parts(current), numeric_parts(latest)) else {
        return Bump::Unknown;
    };
    if a == b {
        return Bump::Unknown;
    }

    // Compare position by position, treating a missing component as 0 so
    // `1.2` and `1.2.0` are the same version rather than incomparable.
    let len = a.len().max(b.len());
    for i in 0..len {
        let (x, y) = (
            a.get(i).copied().unwrap_or(0),
            b.get(i).copied().unwrap_or(0),
        );
        if x == y {
            continue;
        }
        // A DOWNGRADE is not a bump. It happens when a registry yanks a
        // release, and calling it an upgrade would put it in a filtered
        // list as something safe to apply.
        if y < x {
            return Bump::Unknown;
        }
        return match i {
            0 => Bump::Major,
            1 => Bump::Minor,
            // Everything past minor is a patch. .NET's fourth component
            // is a revision, and treating it as its own tier would add a
            // category no filter asks for.
            _ => Bump::Patch,
        };
    }
    Bump::Unknown
}

/// Whether `latest` is strictly newer than `current`.
///
/// `None` means the two are NOT COMPARABLE, and that is a distinct
/// answer from `Some(false)` rather than a failure to produce one. A
/// caller may act on `Some(false)`; it must never act on `None`.
///
/// This exists because a tool's `latest` column is not always newer.
/// `npm outdated --json` reported `jsdom` as `current 30.0.1,
/// latest 29.1.1` while the registry's own `latest` dist-tag was
/// 30.0.1 -- npm appears to report the newest release satisfying an
/// engine constraint it computed, but the column is still labelled
/// `latest`. Passed through, that offers a DOWNGRADE as an update.
///
/// Built on `numeric_parts`, the same function `bump` uses, so the two
/// agree BY CONSTRUCTION rather than by two parsers happening to match.
/// The guarantee callers rely on: `is_newer` answers `Some(false)`
/// exactly where `bump` answers `Unknown` for a backwards jump, and
/// `None` everywhere else `bump` answers `Unknown`. So dropping the
/// `Some(false)` rows can never remove a row `bump` classified as a
/// real upgrade, and never removes an uncomparable one.
///
/// EQUAL numeric parts are `None`, not `Some(false)`. That is the
/// pre-release case: `1.0rc1` and `1.0` have the same numeric parts and
/// the suffix decides an ordering this cannot see, so "we cannot tell"
/// is the honest answer -- the same one `bump` gives. An identical pair
/// (`1.2.3` to `1.2.3`) lands there too, which is right: it is not an
/// update, but it is also not a tool reporting a downgrade, and the
/// rows that legitimately carry `latest == current` (a Terraform
/// provider whose registry lookup failed) must keep showing.
pub fn is_newer(current: &str, latest: &str) -> Option<bool> {
    let (a, b) = (numeric_parts(current)?, numeric_parts(latest)?);
    // Missing components are zero, exactly as in `bump`, so `1.2` and
    // `1.2.0` compare equal rather than as a backwards jump.
    let len = a.len().max(b.len());
    for i in 0..len {
        let (x, y) = (
            a.get(i).copied().unwrap_or(0),
            b.get(i).copied().unwrap_or(0),
        );
        if x != y {
            return Some(y > x);
        }
    }
    None
}

/// The leading dotted numeric components of a version string.
///
/// Returns None when there is nothing comparable, which is what produces
/// `Bump::Unknown` rather than a confident wrong answer.
///
/// Stops at the first component that is not purely numeric, so `1.0rc1`
/// compares as `1.0` -- enough to place the jump, and honest about the
/// rest. A pre-release suffix that changes ordering (`1.0rc1` < `1.0`) is
/// exactly the case where "we cannot tell" beats a guess, and equal
/// numeric parts already answer Unknown.
pub(super) fn numeric_parts(v: &str) -> Option<Vec<u64>> {
    // Strip what people put in front of versions: `v1.2.3`, `^1.2.3`,
    // `~1.2.3`, `>=1.2.3`. PEP 440 epochs (`1!2.0`) are dropped down to
    // the release segment, which is what is comparable across schemes.
    let v = v.trim();
    let v = v.trim_start_matches(['v', 'V', '^', '~', '=', '>', '<', ' ']);
    let v = v.split_once('!').map_or(v, |(_, rest)| rest);
    // Local versions and build metadata are not ordering information.
    let v = v.split(['+', ' ']).next().unwrap_or(v);

    let parts: Vec<u64> = v
        .split('.')
        .map_while(|p| {
            let digits: String = p.chars().take_while(char::is_ascii_digit).collect();
            // A component that does not START with a digit ends the
            // comparable prefix: `1.x` is `1`, not `1.0`.
            if digits.is_empty() || digits.len() != p.len() {
                // Take the numeric head of this component, then stop.
                return digits.parse().ok();
            }
            digits.parse().ok()
        })
        .collect();

    (!parts.is_empty()).then_some(parts)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classifies_the_ordinary_semver_jumps() {
        assert_eq!(bump("1.2.3", "2.0.0"), Bump::Major);
        assert_eq!(bump("1.2.3", "1.3.0"), Bump::Minor);
        assert_eq!(bump("1.2.3", "1.2.4"), Bump::Patch);
    }

    /// .NET ships four-part versions. The fourth is a revision, and it
    /// belongs with patch rather than in a tier of its own that no
    /// filter asks for.
    #[test]
    fn a_dotnet_revision_counts_as_a_patch() {
        assert_eq!(bump("1.2.3.4", "1.2.3.5"), Bump::Patch);
        assert_eq!(bump("1.2.3.4", "1.3.0.0"), Bump::Minor);
    }

    /// Missing components are zero, so `1.2` and `1.2.0` are the same
    /// version rather than incomparable.
    #[test]
    fn a_missing_component_reads_as_zero() {
        assert_eq!(bump("1.2", "1.2.0"), Bump::Unknown, "same version");
        assert_eq!(bump("1.2", "1.2.1"), Bump::Patch);
        assert_eq!(bump("1", "2"), Bump::Major);
    }

    /// The prefixes people actually write.
    #[test]
    fn strips_range_and_v_prefixes() {
        assert_eq!(bump("v1.2.3", "v1.2.4"), Bump::Patch);
        assert_eq!(bump("^1.2.3", "1.3.0"), Bump::Minor);
        assert_eq!(bump("~1.2.3", "2.0.0"), Bump::Major);
    }

    /// PEP 440: epochs and local versions are not ordering information
    /// a cross-ecosystem comparison can use.
    #[test]
    fn handles_pep440_shapes() {
        assert_eq!(bump("1!1.0", "1!2.0"), Bump::Major);
        assert_eq!(bump("1.0+local", "1.1+other"), Bump::Minor);
    }

    /// The third answer, and the reason it exists. A version that cannot
    /// be compared must NOT be guessed: called major it hides from a
    /// "minors only" filter, called minor it is offered as safe.
    #[test]
    fn an_uncomparable_version_is_unknown_not_guessed() {
        assert_eq!(bump("", "1.0.0"), Bump::Unknown);
        assert_eq!(bump("latest", "1.0.0"), Bump::Unknown);
        assert_eq!(bump("1.0.0", "not-a-version"), Bump::Unknown);
        assert_eq!(bump("*", "2.0.0"), Bump::Unknown);
    }

    /// A DOWNGRADE is not a bump. Registries yank releases, and calling
    /// it an upgrade would put it in a filtered list as safe to apply.
    #[test]
    fn a_downgrade_is_never_reported_as_an_upgrade() {
        assert_eq!(bump("2.0.0", "1.9.9"), Bump::Unknown);
        assert_eq!(bump("1.2.3", "1.2.2"), Bump::Unknown);
    }

    /// Equal versions are not an update at all.
    #[test]
    fn an_equal_version_is_not_a_bump() {
        assert_eq!(bump("1.2.3", "1.2.3"), Bump::Unknown);
    }

    /// A pre-release suffix is where "we cannot tell" beats a guess:
    /// `1.0rc1` orders BEFORE `1.0`, which numeric comparison cannot
    /// see. Equal numeric parts already answer Unknown, so this lands in
    /// the right place by construction rather than by accident.
    #[test]
    fn a_prerelease_suffix_does_not_produce_a_confident_answer() {
        assert_eq!(bump("1.0rc1", "1.0"), Bump::Unknown);
    }

    // ---- is_newer -------------------------------------------------
    //
    // The filter `bump` never had. `bump` classifies the SIZE of a jump
    // and answers `Unknown` for anything it cannot place; `is_newer`
    // answers the narrower question of DIRECTION, and its `None` is the
    // line between a row that gets dropped and one that keeps showing.

    #[test]
    fn is_newer_reads_the_ordinary_direction() {
        assert_eq!(is_newer("1.2.3", "2.0.0"), Some(true));
        assert_eq!(is_newer("1.2.3", "1.3.0"), Some(true));
        assert_eq!(is_newer("1.2.3", "1.2.4"), Some(true));
    }

    /// The jsdom case, in its real shape: npm's `latest` column was
    /// OLDER than the version installed.
    #[test]
    fn is_newer_catches_the_backwards_jump_npm_reported() {
        assert_eq!(is_newer("30.0.1", "29.1.1"), Some(false));
        assert_eq!(is_newer("2.0.0", "1.9.9"), Some(false));
        assert_eq!(is_newer("1.2.3", "1.2.2"), Some(false));
    }

    /// The same prefixes `bump` strips, stripped the same way -- they
    /// share `numeric_parts`, and this pins that they agree.
    #[test]
    fn is_newer_strips_range_and_v_prefixes() {
        assert_eq!(is_newer("v1.2.4", "v1.2.3"), Some(false));
        assert_eq!(is_newer("^1.2.3", "1.3.0"), Some(true));
        assert_eq!(is_newer("~2.0.0", "1.0.0"), Some(false));
    }

    /// PEP 440 epochs and local versions, as `bump` handles them: the
    /// epoch drops to the release segment, and the local version is not
    /// ordering information.
    #[test]
    fn is_newer_handles_pep440_shapes() {
        assert_eq!(is_newer("1!2.0", "1!1.0"), Some(false));
        assert_eq!(is_newer("1!1.0", "1!2.0"), Some(true));
        assert_eq!(is_newer("1.1+other", "1.0+local"), Some(false));
    }

    /// .NET's fourth component takes part in the comparison rather than
    /// being ignored, so a revision downgrade is still backwards.
    #[test]
    fn is_newer_compares_four_part_dotnet_versions() {
        assert_eq!(is_newer("1.2.3.5", "1.2.3.4"), Some(false));
        assert_eq!(is_newer("1.2.3.4", "1.2.3.5"), Some(true));
    }

    /// Missing components are zero, so `1.2.0` to `1.2` is not a
    /// backwards jump -- it is the same version.
    #[test]
    fn is_newer_treats_a_missing_component_as_zero() {
        assert_eq!(is_newer("1.2.0", "1.2"), None, "the same version");
        assert_eq!(is_newer("1.2", "1.2.1"), Some(true));
        assert_eq!(is_newer("2", "1"), Some(false));
    }

    /// THE LINE. Nothing comparable means `None`, never `Some(false)`:
    /// a caller that drops backwards rows must not drop these, because
    /// "we cannot tell" is the answer this design deliberately keeps.
    #[test]
    fn an_uncomparable_pair_is_none_not_a_confident_backwards_answer() {
        assert_eq!(is_newer("", "1.0.0"), None, "an empty current");
        assert_eq!(is_newer("1.0.0", ""), None, "an empty latest");
        assert_eq!(is_newer("latest", "1.0.0"), None);
        assert_eq!(is_newer("1.0.0", "not-a-version"), None);
        assert_eq!(is_newer("*", "2.0.0"), None);
        // A bare git revision, which is how a branch-pinned Swift
        // package reports its version.
        assert_eq!(is_newer("f2a1c4d", "1.0.0"), None);
    }

    /// A revision that HAPPENS to start with a digit is the one input
    /// this cannot see through: `numeric_parts` reads `9f2a1c4` as `9`,
    /// so both `is_newer` and `bump` treat it as a version. That is a
    /// property of `numeric_parts` rather than of these two functions,
    /// and it is pinned here so a caller knows not to rely on the
    /// version comparison to recognise a revision.
    ///
    /// Nothing acts on it: Swift is the only ecosystem that reports
    /// revisions, and `run::keep` never filters Swift rows, precisely
    /// because a revision is not a version to compare.
    #[test]
    fn a_revision_starting_with_a_digit_is_not_recognised_as_one() {
        assert_eq!(is_newer("9f2a1c4", "1.0.0"), Some(false));
        assert_eq!(bump("9f2a1c4", "1.0.0"), Bump::Unknown);
    }

    /// A pre-release suffix orders BEFORE the release, which numeric
    /// comparison cannot see -- so it is `None`, not `Some(false)`.
    /// Dropping it as a downgrade would hide a row the user should see.
    #[test]
    fn a_prerelease_suffix_is_uncomparable_not_backwards() {
        assert_eq!(is_newer("1.0rc1", "1.0"), None);
        assert_eq!(is_newer("1.0", "1.0rc1"), None);
    }

    /// An identical pair is not an update, and it is not a downgrade
    /// either. `registry::enrich` deliberately leaves a row at
    /// `latest == current` when a lookup fails, and that row must
    /// survive the filter.
    #[test]
    fn an_equal_pair_is_none_so_a_failed_lookup_still_shows() {
        assert_eq!(is_newer("1.2.3", "1.2.3"), None);
    }

    /// `is_newer` and `bump` share `numeric_parts`, and this pins the
    /// contract the parsers rely on: every pair `is_newer` calls
    /// `Some(false)` is one `bump` calls `Unknown`, and every pair
    /// `bump` places as a real jump is one `is_newer` calls
    /// `Some(true)`.
    #[test]
    fn is_newer_and_bump_never_disagree() {
        let pairs = [
            ("1.2.3", "2.0.0"),
            ("1.2.3", "1.2.2"),
            ("30.0.1", "29.1.1"),
            ("1.0rc1", "1.0"),
            ("1.2.3", "1.2.3"),
            ("latest", "1.0.0"),
            ("1.2.3.4", "1.2.3.5"),
            ("1!2.0", "1!1.0"),
        ];
        for (current, latest) in pairs {
            match is_newer(current, latest) {
                Some(true) => assert_ne!(
                    bump(current, latest),
                    Bump::Unknown,
                    "{current} -> {latest} is newer, so bump must place it"
                ),
                Some(false) | None => assert_eq!(
                    bump(current, latest),
                    Bump::Unknown,
                    "{current} -> {latest} is not an upgrade, so bump must say Unknown"
                ),
            }
        }
    }
}
