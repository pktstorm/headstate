import { useMemo } from "react";
import { HardDrive } from "lucide-react";
import type { Artifact, ArtifactKind } from "@/types/pr";
import { useArtifacts, useArtifactSizes } from "@/api/hooks";
import { formatSize } from "@/lib/worktrees";
import { HelpButton } from "./HelpButton";

/// A placeholder holding the same footprint as the number it stands in
/// for, so rows do not jump as each measurement lands. Matches the one
/// on the worktree page rather than importing it -- that one is local to
/// its module, and a shared component for six lines of markup would be
/// indirection without benefit.
function Skeleton({ className = "" }: { className?: string }) {
  return (
    <span
      // `motion-safe:` and `aria-hidden` for the same reasons the
      // worktree page's version carries them: an animation on every row
      // is what that setting exists to stop, and a placeholder has
      // nothing to announce.
      aria-hidden="true"
      className={`inline-block h-3 rounded bg-[#30363d] align-middle motion-safe:animate-pulse ${className}`}
    />
  );
}

/// What rebuilds each kind, shown beside the row.
///
/// "You can delete this" is only actionable next to what puts it back,
/// and the command is the whole safety argument in one phrase: removal
/// costs a rebuild, not work.
const REBUILD: Record<ArtifactKind, string> = {
  cargo_target: "cargo build",
  node_modules: "npm install",
  terraform: "terraform init",
  build_output: "the project's build",
};

const LABEL: Record<ArtifactKind, string> = {
  cargo_target: "target",
  node_modules: "node_modules",
  terraform: ".terraform",
  build_output: "build output",
};

/// Recently-written directories are probably being built into right now.
///
/// A running `cargo build` does NOT make git dirty -- build output is
/// gitignored -- so no git-based check can see it. Directory mtime is
/// the only available signal, which is why this is surfaced rather than
/// silently folded into a safety verdict.
const ACTIVE_SECS = 60 * 60;

/// Regenerable build output across the scanned directories.
///
/// A separate view from Worktrees despite scanning the same roots,
/// because it answers a different question. Measured on the machine that
/// prompted this: 0.28 GB of Rust build output sat inside worktrees,
/// against 108 GB beside main checkouts -- so the worktree view
/// structurally could not reach 99.7% of the largest thing on the disk.
export function ArtifactsPage() {
  const { data: artifacts = [], isLoading } = useArtifacts(true);
  const { sizes, ages, pending, total } = useArtifactSizes(artifacts, artifacts.length > 0);

  // Largest first, and UNMEASURED rows sort last rather than as zero.
  // Sorting a null as 0 would bury the biggest directory on the machine
  // at the bottom until its size happened to arrive -- the ordering bug
  // #360 describes on the worktree page.
  const rows = useMemo(() => {
    return [...artifacts].sort((a, b) => {
      const sa = sizes.get(a.path);
      const sb = sizes.get(b.path);
      if (sa === undefined && sb === undefined) return a.path.localeCompare(b.path);
      if (sa === undefined) return 1;
      if (sb === undefined) return -1;
      return sb - sa;
    });
  }, [artifacts, sizes]);

  const measured = rows.filter((r) => sizes.has(r.path));
  const totalBytes = measured.reduce((n, r) => n + (sizes.get(r.path) ?? 0), 0);

  if (isLoading) {
    return <p className="p-4 text-sm text-[#8b949e]">Looking for build output…</p>;
  }

  if (artifacts.length === 0) {
    return (
      <p className="p-4 text-sm text-[#8b949e]">
        No build output found in the scanned directories.
      </p>
    );
  }

  return (
    <div className="p-4">
      <div className="mb-3 flex items-center gap-2 text-sm">
        <HardDrive className="h-4 w-4 shrink-0 text-[#8b949e]" aria-hidden="true" />
        <span className="font-semibold text-[#e6edf3]">
          {artifacts.length} director{artifacts.length === 1 ? "y" : "ies"}
        </span>
        {/* "at least" until every batch has answered, because a total
            over a partial set is not the total. Claiming a finished
            number while measurement is still running is the kind of
            quiet wrongness this app tries not to ship. */}
        <span className="text-[#8b949e]">
          {pending > 0 ? "at least " : ""}
          {formatSize(totalBytes)}
        </span>
        {pending > 0 ? (
          <span
            aria-live="polite"
            className="text-xs text-[#58a6ff]"
          >
            measuring — {total - pending} of {total} repositories
          </span>
        ) : null}
        <HelpButton topic="build-artifacts" />
      </div>

      <ul className="flex flex-col gap-1">
        {rows.map((a) => (
          <ArtifactRow
            key={a.path}
            artifact={a}
            bytes={sizes.get(a.path)}
            ageSecs={ages.get(a.path)}
          />
        ))}
      </ul>
    </div>
  );
}

function ArtifactRow({
  artifact,
  bytes,
  ageSecs,
}: {
  artifact: Artifact;
  bytes: number | undefined;
  ageSecs: number | undefined;
}) {
  const active = ageSecs !== undefined && ageSecs < ACTIVE_SECS;
  return (
    <li className="flex items-center gap-3 rounded border border-[#30363d] px-3 py-2 text-sm">
      <span className="shrink-0 rounded-full border border-[#30363d] px-2 py-0.5 text-xs text-[#8b949e]">
        {LABEL[artifact.kind]}
      </span>
      <span className="min-w-0 flex-1 truncate font-mono text-xs text-[#e6edf3]">
        {artifact.path}
      </span>
      {active ? (
        // Surfaced rather than hidden: this is the one hazard git cannot
        // see, and the user is the only one who knows whether a build is
        // theirs.
        <span className="shrink-0 text-xs text-[#d29922]">written recently</span>
      ) : null}
      <span className="shrink-0 text-xs text-[#8b949e]">{REBUILD[artifact.kind]}</span>
      <span className="w-20 shrink-0 text-right tabular-nums">
        {bytes === undefined ? <Skeleton className="ml-auto w-14" /> : formatSize(bytes)}
      </span>
    </li>
  );
}
