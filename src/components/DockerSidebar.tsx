import { Layers } from "lucide-react";
import { type View } from "../store/filters";
import { ViewSwitcher } from "./ViewSwitcher";

/// The Docker view's own sidebar.
///
/// Not a mode of `WorktreeSidebar`: that one lists repositories with
/// worktrees, which has no meaning here.
///
/// It no longer has an AXIS (#852). Docker's was Images versus Builds, and
/// #326 removed the Builds page -- so what remained was one row, rendered
/// as a `<button>` that called `setPanel("list")`, already the only value
/// anything read. A control whose entire effect is to set the state it is
/// already in is not navigation; it is a label that consumes a click and
/// reports back that nothing happened. The `panel` axis is gone with it
/// (see `store/filters.ts`).
export function DockerSidebar({
  viewCounts,
}: {
  viewCounts?: Partial<Record<View, number>>;
}) {
  return (
    <nav className="flex w-64 shrink-0 flex-col border-r border-[#30363d] p-3">
      <ViewSwitcher counts={viewCounts} />
      <div className="min-h-0 flex-1 overflow-y-auto">
        {/* Builds no longer has its own page (#326). Its data was
            diagnostic rather than actionable -- a log with no button on
            it -- and both useful halves now sit where the decision is
            made: a build's duration and cache ratio on the image row it
            produced, and cache health beside the cache it describes.
            
            MEASURED before removing it: of 50 local builds, 41 matched
            a surviving image and the other 9 were superseded builds of
            targets that still appear among those 41. So nothing is
            hidden that the image rows do not already say. */}
        {/* A HEADING, not a button (#852).

            It was a `<button>` with a bare `aria-pressed` -- which is the
            wrong attribute regardless, for the reason
            `SystemHealthSidebar` spells out: "`aria-current="page"` rather
            than `aria-pressed`: these are navigation, not toggles, and a
            screen reader announcing 'pressed' for the page you are already
            on describes a control that did something rather than a
            location you are at." Bare, with no value, it is worse again:
            `aria-pressed` with no value is `aria-pressed="true"`, so it
            announced a permanently-pressed toggle that could never be
            un-pressed.

            With one destination there is nothing to navigate BETWEEN, so
            the honest element is not a better-labelled button but no
            button: this names what the column is showing. Matches
            `SystemHealthSidebar`'s own section heading, which makes the
            same distinction between "changes which view you are in" and
            "says where you are". */}
        <div className="flex w-full items-center gap-2 rounded px-3 py-2 text-sm text-[#e6edf3]">
          <Layers className="h-4 w-4 shrink-0" aria-hidden="true" />
          Images
        </div>
      </div>

      {/* The pinned Stats row is gone (#794). It existed because Stats
          was a `panel` of My PRs and therefore unreachable from here
          without setting both axes at once -- this row's whole job was
          to do that. PR Stats is a view now, so the `ViewSwitcher` above
          reaches it like every other destination, and a second control
          for one of the nine views would be the only view in the app
          with two ways in. */}
    </nav>
  );
}
