import { create } from "zustand";
import { persist } from "zustand/middleware";
import type { Filters } from "../lib/derive";

/// zustand holds UI state only. Server data lives in TanStack Query and is
/// never duplicated here.
/// The three top-level views.
///
/// A separate axis from `panel`: "which view am I in" and "am I looking at
/// the list or the stats" were previously one enum, which is what made the
/// sidebar highlight logic awkward -- `reviewing` and `dashboard` were
/// peers of `list` despite being different kinds of thing.
/// Every top-level view, in sidebar order.
///
/// The single source of truth: `View` is derived from it, and the
/// migration's completeness test iterates it rather than repeating the
/// names. A hardcoded second list is one that gets edited to match
/// whatever the code does and stops checking anything.
export const ALL_VIEWS = [
  "my-prs",
  "to-review",
  "worktrees",
  "branches",
  "docker",
  "artifacts",
  "packages",
  "claude-md",
  // Promoted out of `panel` in #794. It was a sub-page of My PRs,
  // pinned to the sidebar's bottom; it is now a peer view reached from
  // the switcher. Placed before `system-health` because it is still
  // about the user's pull requests, and that entry is deliberately last
  // as the only one that is not.
  "pr-stats",
  "system-health",
] as const;

export type View = (typeof ALL_VIEWS)[number];

/// Views the mobile companion does not offer, whatever is persisted.
///
/// A BUILD-time set, not a viewport one, for the reason `lib/target.ts`
/// gives: "the companion does not offer this" is a statement about which
/// app this is, and hiding by width takes a page away from a desktop
/// user who dragged their window narrow.
///
/// Why this exists at all: until #794, Stats was a `panel` value, and
/// `App.tsx` kept it off the phone by downgrading a stored
/// `panel === "stats"` to `"list"` on every render. Promoting it to a
/// `View` makes that downgrade dead code -- a persisted `view` is read by
/// the switcher, the header and the body route, and patching one of them
/// leaves the other two offering a page the phone cannot show. One set,
/// read everywhere a view is offered or routed.
///
/// `pr-stats` is the only member, and the classification is deliberate
/// rather than inherited: Stats has never been offered on the phone, the
/// companion's first release scoped it out, and #794 moves it between
/// desktop surfaces without changing that. The local-machine views stay
/// on mobile (that is the companion's whole purpose) -- this is not a
/// precedent for hiding them.
export const MOBILE_HIDDEN_VIEWS: ReadonlySet<View> = new Set<View>(["pr-stats"]);

/// The System Health sub-pages, in sidebar order (#687).
///
/// "overview" is the landing page and stays exactly what it was: the
/// panels people open the view for. The rest are DRILL-DOWNS from it,
/// each answering the "why" a panel can only raise -- a panel says
/// memory is at 88%, the Memory page says which processes.
///
/// A separate axis from `panel` rather than three more values in it.
/// `panel` is the My PRs list-versus-stats and Docker images-versus-
/// builds switch; widening it would mean every consumer of `panel` had
/// to know about pages that only exist inside one view, and a health
/// page persisted there would decide what Docker shows. Views that do
/// not have sub-pages should not have to name these.
export const ALL_HEALTH_PAGES = [
  "overview",
  "cpu",
  "memory",
  "disk",
  // Only reachable on a machine with a discoverable GPU (#717). It is
  // in the union unconditionally because the union is the set of pages
  // that EXIST; whether one is offered is a fact about the machine, and
  // `SystemHealthSidebar` filters it out where `gpus` is empty -- the
  // same rule that decides whether the overview draws a GPU panel at
  // all. A page in the union that is not offered is fine; a page
  // offered that renders nothing is not.
  "gpu",
  "network",
  // Battery, thermal and uptime together, and last. None of the three
  // has enough of its own to carry a page: battery is two numbers with
  // no history behind them, thermal is a single coarse label the
  // platform publishes, and uptime is one figure. What they share is
  // that they describe the machine's CONDITION rather than its work,
  // which is a real grouping and not a leftovers drawer -- and it is
  // the drawer test that decided it, since a page per figure would be
  // three sidebar rows leading to one stat each.
  "power",
] as const;

export type HealthPage = (typeof ALL_HEALTH_PAGES)[number];

interface FilterStore {
  /// Filters are PER VIEW: a repo selected in My PRs must not leak into
  /// Worktrees, which has an entirely different repo list.
  filtersByView: Record<View, Filters>;
  view: View;
  /// The sub-page within a view: images versus builds inside Docker. A
  /// separate axis from `view` for the same reason it always was.
  ///
  /// `"stats"` is GONE as of #794 -- Stats is the `pr-stats` view now,
  /// not a sub-page of My PRs. Dropping the value rather than keeping it
  /// unused is the point: a value still in the union is one a component
  /// can set, and a `panel` nobody routes on would be a silent no-op.
  /// The v3 migration below rewrites a persisted `"stats"`.
  panel: "list" | "builds";
  setFilter: <K extends keyof Filters>(key: K, value: Filters[K]) => void;
  applyPreset: (filters: Filters) => void;
  setView: (view: View) => void;
  setPanel: (panel: "list" | "builds") => void;
  /// Which System Health page is open (#687).
  ///
  /// Deliberately NOT persisted, unlike `view` and `panel`. Those
  /// restore what you were working on; this is a drill-down taken to
  /// answer one question, and relaunching straight onto "Memory" would
  /// skip the overview -- the page that says whether there is anything
  /// to drill into today. The landing page has to be the landing page
  /// on launch, or it stops being one.
  healthPage: HealthPage;
  setHealthPage: (page: HealthPage) => void;
  /// How tightly PR rows pack.
  ///
  /// A global preference rather than per-view: it is about the user's
  /// eyes and screen, not about which list they happen to be reading.
  density: "comfortable" | "dense";
  setDensity: (density: "comfortable" | "dense") => void;
  /// The PR the detail view is showing, or null for the list.
  ///
  /// Deliberately NOT persisted: reopening the app on a detail page for a
  /// PR that has since merged is worse than landing on the list.
  selectedPr: { repo: string; number: number } | null;
  selectPr: (pr: { repo: string; number: number } | null) => void;
  /// Rows checked for a bulk action, keyed `repo#number`.
  ///
  /// Keyed rather than held as a list of PRs so selection is independent
  /// of the filtered list: narrowing a filter after selecting must not
  /// silently drop rows from the batch, which the issue calls out as the
  /// first requirement. Not persisted -- a selection is a working set for
  /// one sitting, and restoring it against PRs that may have merged
  /// would be worse than starting empty.
  checked: string[];
  toggleChecked: (key: string) => void;
  /// The last row toggled by a plain click, for shift-click ranges.
  ///
  /// Deliberately NOT persisted and cleared with the view, like
  /// `checked`: an anchor is meaningful only against the list currently
  /// on screen, and restoring one from a previous session would extend a
  /// range from a row the user never touched.
  anchor: string | null;
  /// Which visible row the keyboard cursor is on, or null for none.
  ///
  /// An INDEX into the visible list rather than a PR key, because the
  /// list the user is arrowing through is the filtered, sorted one --
  /// and a key would silently point at a row that filtering has removed.
  /// Cleared with the view for the same reason as `checked`.
  cursor: number | null;
  setCursor: (cursor: number | null) => void;
  setAnchor: (key: string | null) => void;
  setChecked: (keys: string[]) => void;
  clearChecked: () => void;
  reset: () => void;
}

const EMPTY_FILTERS: Record<View, Filters> = {
  "my-prs": {},
  "to-review": {},
  worktrees: {},
  branches: {},
  docker: {},
  artifacts: {},
  packages: {},
  "claude-md": {},
  // PR Stats keeps the repo sidebar (#794), so a repo clicked there
  // writes here -- and `useActiveFilters` reads `[view]` on every render
  // whether or not the page consults the result. `StatsPage` does not
  // consult it yet; the entry is still mandatory, because a missing key
  // is the undefined-crash this record exists to prevent.
  "pr-stats": {},
  // System Health has an entry like every other view even though it
  // has no filters to hold. `filtersByView` must be TOTAL over `View`
  // -- `useActiveFilters` reads `[view]` and every consumer reads
  // `.repo`/`.sort` off the result -- so a view omitted here is the
  // undefined-crash this record exists to prevent, not a saving.
  "system-health": {},
};

/// Filters and view survive a relaunch.
///
/// Every launch previously dropped the user on "All repositories, no
/// filters, sort newest", discarding whatever they were looking at. The
/// state is a flat bag of primitives and string arrays, so `persist` needs
/// no custom serialization.
///
/// `query` is deliberately NOT persisted: a search box that comes back
/// pre-filled with yesterday's text renders a filtered list that looks
/// like an empty one -- the same class of confusion as the old empty
/// state, and harder to diagnose because the cause is offscreen history.
/// The localStorage key this store persists under.
///
/// Exported so the error boundary's reset clears the SAME key this writes.
/// A hardcoded string in both places is one rename away from a reset
/// button that silently clears nothing -- and the crash it exists to
/// escape came from persisted state, so a no-op reset would loop forever.
export const PERSIST_KEY = "headstate-filters";

export const useFilters = create<FilterStore>()(
  persist(
    (set) => ({
      filtersByView: { ...EMPTY_FILTERS },
      view: "my-prs",
      panel: "list",
      density: "comfortable",
      setDensity: (density) => set({ density }),
      setFilter: (key, value) =>
        set((s) => ({
          filtersByView: {
            ...s.filtersByView,
            [s.view]: { ...s.filtersByView[s.view], [key]: value },
          },
          // Choosing a REPO is navigation -- it is how the sidebar
          // changes page -- so it must leave the detail view. Selecting
          // a repository and still staring at one pull request from a
          // different one meant clicking "Back to list" every time, and
          // the sidebar appeared to do nothing.
          //
          // Only `repo`. The other keys narrow the list you are looking
          // at, and closing the detail view on a label filter would
          // throw away what the user is reading.
          ...(key === "repo" ? { selectedPr: null } : {}),
        })),
      // Preset navigation replaces the filter set wholesale, so a click
      // never inherits a filter the user forgot was active and shows a
      // count that doesn't match the list it opens.
      applyPreset: (filters) =>
        set((s) => ({
          filtersByView: { ...s.filtersByView, [s.view]: filters },
          panel: "list",
      density: "comfortable",
      setDensity: (density) => set({ density }),
        })),
      // Selection clears with the view: a working set assembled on My
      // PRs means nothing on the review list, and carrying it across
      // would let a later batch act on rows the user cannot see.
      setView: (view) =>
        set({
          view,
          selectedPr: null,
          checked: [],
          anchor: null,
          cursor: null,
          // Leaving System Health and coming back lands on the
          // overview, for the same reason it is not persisted: the
          // drill-down answered a question that is now behind you, and
          // returning to a detail page skips the one that says whether
          // there is a new question worth asking.
          healthPage: "overview",
        }),
      setPanel: (panel) => set({ panel }),
      healthPage: "overview",
      setHealthPage: (healthPage) => set({ healthPage }),
      selectedPr: null,
      selectPr: (selectedPr) => set({ selectedPr }),
      checked: [],
      anchor: null,
      cursor: null,
      setCursor: (cursor) => set({ cursor }),
      setAnchor: (anchor) => set({ anchor }),
      toggleChecked: (key) =>
        set((s) => ({
          checked: s.checked.includes(key)
            ? s.checked.filter((k) => k !== key)
            : [...s.checked, key],
        })),
      setChecked: (checked) => set({ checked }),
      clearChecked: () => set({ checked: [] }),
      // `repo` is sidebar NAVIGATION, not a filter chip -- it decides
      // which page you are on, scopes the priorities strip, and
      // pre-answers the wizard's repo step. Clearing it navigated the user
      // off the repo they were looking at, which is not what "Clear
      // filters" says it does.
      reset: () =>
        set((s) => {
          const current = s.filtersByView[s.view];
          return {
            filtersByView: {
              ...s.filtersByView,
              [s.view]: current.repo ? { repo: current.repo } : {},
            },
          };
        }),
    }),
    {
      name: PERSIST_KEY,
      // Bumped when the persisted SHAPE changes. Without this, a store
      // saved by v1 -- which had a flat `filters` and a `view` enum
      // conflating view with panel -- rehydrates straight into the new
      // shape, leaving `filtersByView` undefined and crashing on first
      // render. Tests never caught it because they always start empty.
      //
      // v3 (#794): Stats stopped being a `panel` value and became the
      // `pr-stats` VIEW. A store written by v2 can hold
      // `panel: "stats"`, which now routes nowhere -- such a user would
      // land on the PR list with no sign their Stats page had moved.
      version: 3,
      migrate: (persisted: unknown, from: number) => {
        // Run in ORDER and fall through, rather than one branch per
        // starting version. A v1 store that sat unopened across both
        // changes has to go v1 -> v2 -> v3; a chain of
        // `if (from === n)` arms would apply one and skip the other,
        // which is exactly the black-window class of bug the comment
        // below this is about.
        let state = persisted ?? {};
        if (from < 2) {
          // v1 -> v2: lift the single filter set into the active view,
          // and split `view` into view + panel. An unrecognised value
          // falls back to the defaults rather than propagating a bad
          // state.
          const old = state as { filters?: Filters; view?: string };
          // "dashboard" was v1's name for the stats page. v2 mapped it
          // onto `panel: "stats"`; since #794 the destination is the
          // view itself, so the hop through `panel` is gone and this
          // lands where v3 would have put it anyway.
          const view: View =
            old.view === "dashboard"
              ? "pr-stats"
              : old.view === "reviewing"
                ? "to-review"
                : "my-prs";
          state = {
            // The old flat filters belong to the view they were
            // filtering. "dashboard" had none of its own -- it showed
            // the whole account -- but `pr-stats` keeps the repo
            // sidebar, so carrying them there is what the user had.
            filtersByView: { ...EMPTY_FILTERS, [view]: old.filters ?? {} },
            view,
            panel: "list",
          };
        }
        if (from < 3) {
          // v2 -> v3: a stored `panel: "stats"` meant "My PRs, showing
          // the stats page". That destination is now a view, so move
          // the user THERE and reset `panel` to the only value My PRs
          // and Docker still share. Left as "stats" it would be a value
          // no route reads, and the user would silently lose the page
          // they closed the app on.
          const old = state as { panel?: string; view?: string };
          if (old.panel === "stats") {
            state = {
              ...old,
              // Only from My PRs. `panel` is shared with Docker, and a
              // Docker user cannot have set "stats" -- but a store hand-
              // edited or written by a build mid-rename could, and
              // teleporting someone off Docker would be worse than
              // dropping a value that was never reachable there.
              ...(old.view === "my-prs" || old.view === undefined
                ? { view: "pr-stats" }
                : {}),
              panel: "list",
            };
          }
        }
        return state as never;
      },
      // Stored state is REPLACED into the store, not merged, so adding a
      // view to the `View` union silently breaks every existing install:
      // the persisted `filtersByView` has no key for it, and reading
      // `.sort` off undefined takes down the entire app with a black
      // window. This happened for real when `worktrees` and `docker`
      // were added -- both were already version 2, so the migration
      // above returned the old shape untouched.
      //
      // Merging per-view against EMPTY_FILTERS makes the store complete
      // by construction, for every view that exists now or later.
      merge: (persisted, current) => {
        const p = (persisted ?? {}) as Partial<FilterStore>;
        return {
          ...current,
          ...p,
          filtersByView: { ...EMPTY_FILTERS, ...(p.filtersByView ?? {}) },
        };
      },
      partialize: (s) => ({
        // `query` is dropped per view for the same reason as before: a
        // search box restored with yesterday's text renders a filtered
        // list that looks like an empty one.
        filtersByView: Object.fromEntries(
          Object.entries(s.filtersByView).map(([k, f]) => [k, { ...f, query: undefined }]),
        ) as Record<View, Filters>,
        view: s.view,
        panel: s.panel,
        density: s.density,
      }),
    },
  ),
);

/// Shared empty-filter object, so the fallback below is reference-stable.
const NO_FILTERS: Filters = Object.freeze({});

/// The active view's filters.
///
/// A selector rather than a stored field, so there is exactly one source
/// of truth and no chance of the two drifting apart.
export function useActiveFilters(): Filters {
  // NO_FILTERS is a module constant, not an inline `?? {}`. zustand
  // compares selector results by reference, so returning a fresh `{}`
  // each call makes every read look like a change and spins
  // useSyncExternalStore into an infinite re-render -- a worse failure
  // than the crash this guards against. Caught by the test below.
  //
  // The guard itself is real, not defensive noise. `filtersByView` is
  // rehydrated from disk, and `persist` REPLACES this object rather than
  // merging it -- so a store written before a view existed comes back
  // without that view's key. Every consumer reads `.sort` / `.repo` off
  // this value, so returning undefined crashes the whole tree.
  return useFilters((s) => s.filtersByView[s.view] ?? NO_FILTERS);
}
