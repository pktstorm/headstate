import type { Ecosystem } from "@/types/pr";

/// The one ecosystem a selection shares, or null when they differ.
///
/// Mirrors `packages::apply::sole_ecosystem`. A mixed selection is named
/// for no ecosystem at all rather than for whichever row came first --
/// the wizard's checkboxes are per row, so ticking a crate and a pip
/// package together is ordinary, and picking one of the two would be
/// arbitrary and wrong half the time.
export function soleEcosystem(ecosystems: Ecosystem[]): Ecosystem | null {
  const first = ecosystems[0];
  if (first === undefined) return null;
  return ecosystems.every((e) => e === first) ? first : null;
}

/// The UTC stamp a generated branch name ends with: `YYYYMMDD-HHMMSS`.
///
/// Mirrors `STAMP_FORMAT` in `packages::apply`, which is chrono's
/// `%Y%m%d-%H%M%S`. Built from the `getUTC*` getters, NOT the local ones:
/// the desktop derives this again when the run starts, and a companion in
/// another timezone would otherwise predict a different name for the same
/// instant. Every field is padded to its fixed width, or the stamp
/// changes length and `git branch` stops sorting chronologically.
function stamp(at: Date): string {
  const p = (n: number, width = 2) => String(n).padStart(width, "0");
  return (
    `${p(at.getUTCFullYear(), 4)}${p(at.getUTCMonth() + 1)}${p(at.getUTCDate())}` +
    `-${p(at.getUTCHours())}${p(at.getUTCMinutes())}${p(at.getUTCSeconds())}`
  );
}

/// The branch name an update run will use, mirroring the backend.
///
/// #409 asked for the name to auto-populate and stay overridable, so
/// the field has to show what WOULD be used before the run starts.
/// This mirrors `packages::apply::branch_name_at` -- a second
/// implementation, which is a real cost, but the alternative is a round
/// trip to derive a string the user is about to edit.
///
/// A test asserts the two agree on the cases that matter; if they ever
/// diverge, the field shows one name and the run uses another, which is
/// worse than not offering the field at all.
///
/// # The ecosystem and the clock (#797)
///
/// The name was `headstate/updates-<package count>`, which collided
/// whenever two runs in one repository touched the same number of
/// packages -- and `create_worktree` refuses an existing branch, so the
/// second run failed. It is now
/// `headstate/<ecosystem>-deps-<UTC stamp>`, which is unique per run and
/// says what the run was. The ecosystem is passed IN rather than fetched,
/// which is the whole point: every row the wizard renders already carries
/// one, so the field can still be predicted with no round trip.
///
/// `at` is passed in too, and that is what makes the prediction BINDING
/// rather than advisory. Both sides read a clock, and two clocks read a
/// second apart give two different names -- so the wizard derives the
/// name once and SENDS it as the override (see `UpdateWizard`), instead of
/// showing one string and letting the backend derive another. The
/// parameter is also what lets the agreement test compare the two at a
/// fixed instant.
///
/// `_names` is unused and kept deliberately: every caller has the list,
/// and a name that wants a package in it again should not need the call
/// sites rewritten. Underscored to say so, matching `_packages` on the
/// Rust side.
export function derivedBranchName(
  _names: string[],
  ecosystem: Ecosystem | null,
  at: Date,
): string {
  // No sanitising. An ecosystem slug is ASCII lowercase and the stamp is
  // digits and a dash, so unlike the package names the old name
  // interpolated, nothing here can produce a character git refuses.
  //
  // Assembled as "<what>-<stamp>" rather than as two templates, so a
  // mixed run cannot leave `headstate/-deps-…` -- a leading dash git
  // reads as an option, and `branchNameError` refuses.
  const what = ecosystem === null ? "deps" : `${ecosystem}-deps`;
  return `headstate/${what}-${stamp(at)}`;
}

/// Whether a user-supplied branch name is one git will accept.
///
/// Mirrors `packages::apply::valid_branch_name`. Checked here so the
/// field can say what is wrong as it is typed; the backend checks again
/// regardless, because this is convenience and that is the gate.
export function branchNameError(name: string): string | null {
  if (name === "") return "Enter a branch name";
  if (name.startsWith("-")) return "Cannot start with '-'";
  if (name.startsWith("/") || name.endsWith("/") || name.includes("//"))
    return "Cannot start or end with '/', or contain '//'";
  if (name.endsWith(".") || name.includes("..")) return "Cannot end with '.' or contain '..'";
  if (name.endsWith(".lock")) return "Cannot end with '.lock'";
  if (name.includes("@{")) return "Cannot contain '@{'";
  const controlOrSpace = [...name].some(
    (c) => c === " " || c.codePointAt(0)! < 0x20 || c.codePointAt(0)! === 0x7f,
  );
  if (controlOrSpace) return "Cannot contain spaces or control characters";
  const bad = [...name].find((c) => "~^:?*[\\".includes(c));
  if (bad) return `Cannot contain '${bad}'`;
  return null;
}
