import type { PullRequest } from "@/types/pr";
import { type View, useActiveFilters, useFilters } from "@/store/filters";
import { ViewSwitcher } from "@/components/ViewSwitcher";
import { repoCounts } from "@/lib/repos";

/// Repos where the user currently has open PRs, busiest first, plus an
/// always-first "All repositories" entry that is the default (no `repo`
/// filter set). Selecting a repo writes through the shared filter store --
/// this component holds no filter state of its own.
///
/// This is the sidebar for My PRs AND for PR Stats (#794). Stats used to
/// own a pinned row at the bottom of this column, outside the scroll area
/// so it stayed reachable however many repos the list grew to. That row
/// is gone: the destination moved into `ViewSwitcher`, which is where the
/// rest of the app's navigation already lived, and a single column of
/// repositories with nothing pinned under it is the simpler layout the
/// old one was working around.
///
/// The rows stay live on both views, and highlight on either. On PR Stats
/// a selection writes to that view's own filter set and nothing reads it
/// yet -- `StatsPage` is a whole-account summary -- so the highlight is
/// the honest thing to render: it says what was clicked. `ViewSwitcher`
/// carries why the column is here at all rather than blank.
export function RepoSidebar({
  prs,
  viewCounts,
}: {
  prs: PullRequest[];
  /// Badge counts for the switcher, e.g. how many PRs await review.
  viewCounts?: Partial<Record<View, number>>;
}) {
  const filters = useActiveFilters();
  const { setFilter, view } = useFilters();
  const counts = repoCounts(prs);

  const rowClass = (active: boolean) =>
    `flex w-full items-center justify-between rounded px-3 py-2 text-sm ${
      active ? "bg-[#1f6feb] text-white" : "text-[#e6edf3] hover:bg-[#161b22]"
    }`;

  // Both views this sidebar serves, not just My PRs (#794). The repo rows
  // are the navigation for each, so a selection has to look selected on PR
  // Stats too -- and there is no longer a pinned Stats row competing for
  // the highlight, which is what the old `panel === "list"` half of this
  // was avoiding.
  //
  // Still guarded on `view` at all, because this component is the
  // FALLBACK sidebar in `App.tsx`: any future view that falls through to
  // it would otherwise show a repo row highlighted for a page that never
  // reads `filters.repo`.
  const repoActive = view === "my-prs" || view === "pr-stats";

  return (
    <nav className="flex w-64 shrink-0 flex-col border-r border-[#30363d] p-3">
      <ViewSwitcher counts={viewCounts} />
      <div className="min-h-0 flex-1 overflow-y-auto">
        <button
          type="button"
          onClick={() => setFilter("repo", undefined)}
          className={rowClass(repoActive && !filters.repo)}
        >
          <span>All repositories</span>
          <span>{prs.length}</span>
        </button>
        {counts.map(({ repo, count }) => (
          <button
            type="button"
            key={repo}
            onClick={() => setFilter("repo", repo)}
            className={rowClass(repoActive && filters.repo === repo)}
          >
            <span className="truncate">{repo}</span>
            <span className="ml-2 shrink-0">{count}</span>
          </button>
        ))}
      </div>
    </nav>
  );
}
