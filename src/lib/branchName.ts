/// The branch name an update run will use, mirroring the backend.
///
/// #409 asked for the name to auto-populate and stay overridable, so
/// the field has to show what WOULD be used before the run starts.
/// This mirrors `packages::apply::branch_name` -- a second
/// implementation, which is a real cost, but the alternative is a round
/// trip to derive a string the user is about to edit.
///
/// A test asserts the two agree on the cases that matter; if they ever
/// diverge, the field shows one name and the run uses another, which is
/// worse than not offering the field at all.
export function derivedBranchName(names: string[]): string {
  const sanitise = (s: string) =>
    [...s].map((c) => (/[A-Za-z0-9\-.]/.test(c) ? c : "-")).join("");
  if (names.length === 0) return "headstate/updates";
  if (names.length === 1) return `headstate/update-${sanitise(names[0])}`;
  return `headstate/updates-${names.length}`;
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
