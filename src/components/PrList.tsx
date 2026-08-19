import type { PullRequest } from "@/types/pr";
import { PrRow } from "@/components/PrRow";

export function PrList({ prs }: { prs: PullRequest[] }) {
  // Newest first, as specified.
  const sorted = [...prs].sort(
    (a, b) => new Date(b.created_at).getTime() - new Date(a.created_at).getTime(),
  );

  return (
    <div className="rounded-md border border-[#30363d]">
      <div className="flex items-center justify-between border-b border-[#30363d] bg-[#161b22] px-4 py-3 text-sm">
        <span className="font-semibold text-[#e6edf3]">{sorted.length} Open</span>
      </div>
      {sorted.length === 0 ? (
        <div className="px-4 py-12 text-center text-sm text-[#8b949e]">
          No pull requests match these filters.
        </div>
      ) : (
        sorted.map((pr) => <PrRow key={`${pr.repo}#${pr.number}`} pr={pr} />)
      )}
    </div>
  );
}
