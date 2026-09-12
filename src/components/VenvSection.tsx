import { isCancelled } from "@/lib/cancelled";
import { ActingOnDesktop } from "./ActingOnDesktop";
import { useState } from "react";
import { toast } from "sonner";
import type { Venv, VenvState } from "@/types/pr";
import { useRemoveVenvs, useVenvs, useVenvSizes } from "@/api/hooks";
import { formatSize } from "@/lib/worktrees";
import { relativeSeconds } from "@/lib/time";
import { useIsMobile } from "@/lib/useIsMobile";
import { Dialog, DialogContent, DialogTitle } from "./ui/dialog";
import { HelpButton } from "./HelpButton";
import { QueryError, errorMessage } from "./QueryError";

/// How long idle counts as stale, mirroring `STALE_SECS` in Rust.
///
/// Duplicated rather than plumbed through because it is only used to
/// LABEL rows here; the backend's value is the one that decides anything,
/// and this never gates a removal.
const STALE_SECS = 90 * 24 * 60 * 60;

/// Whether a venv is offered for removal at all.
///
/// Orphans always; stale only when the user has said so in Settings.
///
/// An orphan is a FACT -- the path that made it is gone, so nothing can
/// ever use it again. A stale venv is a JUDGEMENT about a project that
/// still exists, which is why it needs an explicit opt-in rather than
/// being removable by default.
///
/// That opt-in already existed: "Also allow removing stale virtualenvs"
/// in Settings, which `remove_venvs` reads as `policy.allow_stale`. The
/// BACKEND honoured it and this function did not, so turning the setting
/// on changed nothing a user could see -- the checkbox stayed disabled
/// and the row could never be selected. `live` is never removable at
/// either layer.
function isRemovable(v: Venv, state: VenvState): boolean {
  if (v.path.length === 0) return false;
  // Orphaned OR stale. No setting.
  //
  // Stale used to require `remove_stale_venvs`, on the reasoning that a
  // 90-day threshold is a guess about intent. True for AUTOMATIC
  // cleanup; wrong here. Ticking a specific row and confirming in a
  // dialog IS the intent, and no other artifact asks twice -- a Rust
  // `target` costs minutes to rebuild and has no gate, while a
  // virtualenv is `poetry install`.
  //
  // `live` stays unremovable: its project exists and is in use, which
  // is a fact rather than a threshold.
  return state === "orphaned" || state === "stale";
}

/// The state a row displays, once its idle time is known.
///
/// An orphan stays an orphan however recently it was touched: its path
/// is gone, so mtime says nothing about whether anyone wants it.
function displayState(v: Venv, idleSecs: number | undefined): VenvState {
  if (v.state === "orphaned") return "orphaned";
  // An idle time cannot complete an incomplete scan. Ageing `unknown`
  // into `stale` here would make it removable again, undoing the
  // suppression the backend applied for exactly that reason (#747).
  if (v.state === "unknown") return "unknown";
  if (idleSecs !== undefined && idleSecs >= STALE_SECS) return "stale";
  return v.state;
}

const TONE: Record<VenvState, string> = {
  orphaned: "bg-[#f85149]/15 text-[#f85149]",
  stale: "bg-[#d29922]/15 text-[#d29922]",
  live: "bg-[#238636]/15 text-[#3fb950]",
  // Grey, deliberately: the danger tones say "act on this", and the
  // whole point of `unknown` is that this run cannot tell you to.
  unknown: "bg-[#8b949e]/15 text-[#8b949e]",
};

/// Poetry virtualenvs, on the Artifacts page.
///
/// Here rather than in its own view because it answers the same question
/// — where did the disk go — and splitting "build output" from "tool
/// caches" across two views would make a user check two places for one
/// answer.
export function VenvSection() {
  const isMobile = useIsMobile();
  // `isLoading` as well as the data: the `= []` default made "still
  // scanning" and "there are none" the SAME value, and the early
  // return below then removed the section entirely. On a real machine
  // the scan takes 26 SECONDS -- measured, walking 28,144 directories --
  // so the page was indistinguishable from one with no virtualenvs for
  // almost half a minute, which is exactly how it was reported.
  //
  // And `isError` too (#846). This is the sharpest case in that issue,
  // because it was ALREADY FIXED ONCE for the adjacent bug and the fix
  // did not carry: `isLoading` was separated from empty, `isError` was
  // not. So a rejected scan still left `venvs` at `[]` and the
  // `venvs.length === 0` return below still removed the entire section --
  // its orphan count, its bulk-remove button, all of it -- with no error
  // and nothing to retry. Worse than the 26-second wait this comment was
  // written about, because a wait ends.
  //
  // Aggravated by the hook's own settings: `staleTime: 30 * 60 * 1000`
  // and `refetchOnWindowFocus: false` are right for data and pinned a
  // FAILURE for half an hour. `useVenvs` now carries `retry: false` and
  // this renders an explicit retry, which is the pairing
  // `useStatsBoard`'s rule requires.
  const { data: venvs = [], isLoading, isError, error, refetch } = useVenvs(true);
  const { sizes, idle, measuring, pending, total } = useVenvSizes(venvs, venvs.length > 0);
  const [checked, setChecked] = useState<Set<string>>(new Set());
  const [confirming, setConfirming] = useState(false);
  const [busy, setBusy] = useState(false);
  const remove = useRemoveVenvs();

  // Say the scan is running rather than rendering nothing. The
  // artifacts list already does this; this section did not.
  if (isLoading) {
    return (
      <p className="px-1 py-2 text-xs text-[#8b949e]" aria-live="polite">
        Looking for Poetry virtualenvs…
      </p>
    );
  }
  // BEFORE the `return null` below, which is what used to swallow it
  // (#846). An error arm placed after that line would be unreachable in
  // exactly the case it exists for, since a rejection leaves `venvs` at
  // `[]`.
  //
  // Keeps the section's HEADING, unlike the empty case. A bare error
  // panel floating under the artifact list would not say what failed; the
  // section is how the reader knows this is about virtualenvs and not
  // about the build output above it. Absence is what the empty state is
  // for, and a failure is not an absence.
  if (isError) {
    return (
      <section className="mt-6">
        <div className="mb-3 flex items-center gap-2 text-sm">
          <span className="font-semibold text-[#e6edf3]">Poetry virtualenvs</span>
          <HelpButton topic="poetry-venvs" />
        </div>
        <QueryError
          title="Could not look for Poetry virtualenvs"
          message={errorMessage(error)}
          onRetry={() => void refetch()}
        >
          {/* Says what the failure COSTS, which matters more here than
              anywhere else on the page: the orphan count is the number the
              user acts on, and its absence is not a zero. */}
          <p className="mx-auto mt-2 max-w-lg text-sm text-[#8b949e]">
            Nothing was scanned, so the orphan count is unknown — not zero.
          </p>
        </QueryError>
      </section>
    );
  }
  // Only once the scan has ANSWERED does an empty list mean "none".
  if (venvs.length === 0) return null;

  const rows = [...venvs]
    .map((v) => ({ v, state: displayState(v, idle.get(v.path)) }))
    .sort((a, b) => (sizes.get(b.v.path) ?? 0) - (sizes.get(a.v.path) ?? 0));

  const orphans = rows.filter((r) => r.state === "orphaned");
  const orphanBytes = orphans.reduce((n, r) => n + (sizes.get(r.v.path) ?? 0), 0);
  // A truncated project walk suppresses the orphan verdict for the whole
  // run, so `unknown` rows are the SIGNAL that the count above is not the
  // real one -- there may be orphans hiding among them (#747).
  const unknown = rows.filter((r) => r.state === "unknown");
  const selectedBytes = [...checked].reduce((n, p) => n + (sizes.get(p) ?? 0), 0);

  return (
    <section className="mt-6">
      <div className="mb-3 flex items-center gap-2 text-sm">
        <span className="font-semibold text-[#e6edf3]">Poetry virtualenvs</span>
        <span className="text-[#8b949e]">
          {orphans.length} orphaned{measuring ? "" : ` · ${formatSize(orphanBytes)}`}
        </span>
        {measuring ? (
          // COUNTED, not a bare "measuring…". Sizing is chunked now, so
          // there is real progress to report -- and a bare word on a
          // pass that took 73 seconds is indistinguishable from being
          // stuck, which is how it was reported.
          <span aria-live="polite" className="text-xs text-[#58a6ff]">
            measuring — {total - pending} of {total}
          </span>
        ) : null}
        <HelpButton topic="poetry-venvs" />

        {/* One click for the whole provable set. With 78 orphans across
            one deleted project, ticking them individually is 78 clicks
            for a decision the user makes once -- and every one of them
            is a fact rather than a judgement, so there is nothing to
            weigh row by row. */}
        {orphans.length > 1 && checked.size === 0 ? (
          <button
            type="button"
            disabled={busy}
            onClick={() => {
              setChecked(new Set(orphans.map((r) => r.v.path)));
              setConfirming(true);
            }}
            className="ml-auto rounded border border-[#f85149]/40 px-2 py-0.5 text-xs text-[#f85149] hover:bg-[#f85149]/10 disabled:opacity-50"
          >
            Remove all {orphans.length} orphaned
            {measuring ? "" : ` · ${formatSize(orphanBytes)}`}
          </button>
        ) : null}

        {checked.size > 0 ? (
          <button
            type="button"
            disabled={busy}
            onClick={() => setConfirming(true)}
            className="ml-auto rounded border border-[#f85149]/40 px-2 py-0.5 text-xs text-[#f85149] hover:bg-[#f85149]/10 disabled:opacity-50"
          >
            {busy ? "Removing…" : `Remove ${checked.size} · ${formatSize(selectedBytes)}`}
          </button>
        ) : null}
      </div>

      {/* Says the answer is incomplete, in the one place the answer is
          read. The old behaviour returned a short list with no marker at
          all, so a run that had stopped walking early looked exactly
          like one that found nothing -- and "0 orphaned" is a claim, not
          an absence of one (#747). */}
      {unknown.length > 0 ? (
        <p
          role="status"
          className="mb-3 rounded border border-[#d29922]/40 bg-[#d29922]/10 px-3 py-2 text-xs text-[#d29922]"
        >
          The project scan did not finish, so {unknown.length} virtualenv
          {unknown.length === 1 ? "" : "s"} could not be checked. They are shown as
          unknown and cannot be removed — some may in fact be orphaned. Narrowing the
          scanned directories in Settings will let the scan complete.
        </p>
      ) : null}

      {confirming ? (
        <Dialog open onOpenChange={(o) => !o && setConfirming(false)}>
          <DialogContent className="max-w-lg">
            <DialogTitle>
              Remove {checked.size} virtualenv{checked.size === 1 ? "" : "s"}?
            </DialogTitle>
            <ActingOnDesktop />
            <p className="mt-3 text-sm text-[#e6edf3]">
              This frees {formatSize(selectedBytes)}. Every one of these belongs to a
              project directory that no longer exists, so nothing can use them again.
            </p>
            <div className="mt-5 flex justify-end gap-2">
              <button
                type="button"
                onClick={() => setConfirming(false)}
                className="rounded border border-[#30363d] px-3 py-1.5 text-sm hover:bg-[#21262d]"
              >
                Cancel
              </button>
              <button
                type="button"
                onClick={() => {
                  const paths = [...checked];
                  setConfirming(false);
                  setBusy(true);
                  remove(paths).then(
                    (outcomes) => {
                      setBusy(false);
                      // Clear only what was actually REMOVED.
                      //
                      // A blanket reset discarded two things: anything
                      // ticked while the removal was in flight (a long
                      // window, with no sign it happened), and the
                      // selection for rows that FAILED -- which are
                      // exactly the ones still needing attention.
                      setChecked((prev) => {
                        const next = new Set(prev);
                        for (const o of outcomes) {
                          if (o.error === null) next.delete(o.path);
                        }
                        return next;
                      });
                      const failed = outcomes.filter((o) => o.error !== null);
                      const ok = outcomes.length - failed.length;
                      if (failed.length === 0) {
                        toast.success(`Removed ${ok} virtualenv${ok === 1 ? "" : "s"}`);
                      } else {
                        toast.error(
                          `${failed.length} of ${outcomes.length} could not be removed`,
                          { description: failed.map((f) => f.error).join("\n") },
                        );
                      }
                    },
                    (e: unknown) => {
                      setBusy(false);
                      // Silent when the user dismissed the biometric prompt:
                      // they declined, nothing was removed, and reporting
                      // their own decision back as a failure is noise.
                      if (isCancelled(e)) return;
                      toast.error("The removal could not run", {
                        description: typeof e === "string" ? e : undefined,
                      });
                    },
                  );
                }}
                className="rounded bg-[#da3633] px-3 py-1.5 text-sm font-medium text-white hover:bg-[#c93c37]"
              >
                Remove
              </button>
            </div>
          </DialogContent>
        </Dialog>
      ) : null}

      <ul className="flex flex-col gap-1">
        {rows.map(({ v, state }) => {
          const removable = isRemovable(v, state);
          // The cells, built once. The desktop lays them out on one
          // line; the phone puts the checkbox, project and size first
          // and the verdict, its evidence and the age beneath.
          const checkbox = (
              <input
                type="checkbox"
                checked={checked.has(v.path)}
                // Only orphans are selectable. A disabled control that
                // explains itself beats one that silently ignores clicks.
                disabled={!removable}
                onChange={() =>
                  setChecked((prev) => {
                    const next = new Set(prev);
                    if (next.has(v.path)) next.delete(v.path);
                    else next.add(v.path);
                    return next;
                  })
                }
                aria-label={
                  removable
                    ? `Select ${v.project} virtualenv`
                    : // The REASON differs, and a disabled control that
                      // explains itself has to explain the right thing:
                      // `unknown` is not "in use", it is "not checked".
                      state === "unknown"
                      ? `${v.project} virtualenv cannot be removed: the project scan did not finish, so it could not be checked`
                      : `${v.project} virtualenv cannot be removed: its project still exists and is in use`
                }
                className="shrink-0 disabled:opacity-30"
              />
          );
          const project = (
              <span className={isMobile ? "min-w-0 flex-1 truncate font-semibold text-[#e6edf3]" : "shrink-0 font-semibold text-[#e6edf3]"}>{v.project}</span>
          );
          const verdict = (
              <span className={`shrink-0 rounded-full px-2 py-0.5 text-xs ${TONE[state]}`}>
                {state}
              </span>
          );
          const source = (
              /* The SOURCE is the evidence for the verdict. For a live
                  or stale venv it names the directory that still exists,
                  which is what lets someone disagree with the label. */
              <span className="min-w-0 flex-1 truncate font-mono text-xs text-[#8b949e]">
                {v.source ?? "no project directory found"}
              </span>
          );
          const age = (
              /* AGE, for the same reason the artifacts list got it in
                  #417: size cannot rank these rows. The idle time was
                  already being fetched and used ONLY to compute the
                  stale badge -- the number itself was never shown, so
                  "is this old enough to delete" had no answer on screen.

                  Unknown renders as an em dash, never as "just now":
                  reading not-yet-measured as brand new would hide
                  exactly the venvs worth removing. */
              <span className={isMobile ? "shrink-0 text-xs text-[#8b949e]" : "w-24 shrink-0 text-right text-xs text-[#8b949e]"}>
                {idle.has(v.path) ? relativeSeconds(idle.get(v.path) ?? 0) : "—"}
              </span>
          );
          const size = (
              <span className="w-20 shrink-0 text-right text-xs tabular-nums text-[#8b949e]">
                {sizes.has(v.path) ? formatSize(sizes.get(v.path) ?? 0) : "—"}
              </span>
          );
          if (isMobile) {
            return (
              <li
                key={v.path}
                className="flex flex-col gap-1 rounded border border-[#30363d] px-3 py-2 text-sm"
              >
                <div className="flex items-center gap-3">
                  {checkbox}
                  {project}
                  {size}
                </div>
                <div className="flex items-center gap-3 pl-7">
                  {verdict}
                  {source}
                  {age}
                </div>
              </li>
            );
          }
          return (
            <li
              key={v.path}
              className="flex items-center gap-3 rounded border border-[#30363d] px-3 py-2 text-sm"
            >
              {checkbox}
              {project}
              {verdict}
              {source}
              {age}
              {size}
            </li>
          );
        })}
      </ul>
    </section>
  );
}
