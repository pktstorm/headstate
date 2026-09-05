import { useEffect, useRef, useState } from "react";
import { usePairingRequest, useRespondToPairing } from "../api/hooks";
import type { PairingRequest } from "../api/tauri";
import { mmss, useCountdown } from "@/lib/countdown";
import { formatFingerprint } from "@/lib/fingerprint";
import { Dialog, DialogContent, DialogTitle } from "./ui/dialog";

/// Matches `DECISION_TIMEOUT` in `remote/pairing.rs`. The Rust side
/// denies on its own at this point; the modal denies a moment earlier
/// so the user sees the door close rather than an Approve that fails.
const DECISION_SECS = 120;

/// The Rust side's `RespondError::NameTaken`, as it reaches the webview:
/// `a device named "…" is already paired`. Matched loosely on purpose --
/// the wording may be tuned, the situation will not.
const nameTaken = (e: unknown) => typeof e === "string" && /already paired/i.test(e);

/// The confirmation the desktop shows when a phone has proved it holds
/// the pairing token. Mounted once at the app root by
/// [`PairingRequestModal`]; this is the dialog itself, with the request
/// passed in so tests can drive it directly.
///
/// Deny is the safe default everywhere: on timeout, on Escape, on
/// closing the dialog. Approve is the only path that stores anything.
export function PairingRequestDialog({
  request,
  onDone,
}: {
  request: PairingRequest;
  onDone: () => void;
}) {
  const respond = useRespondToPairing();
  const [stage, setStage] = useState<"decide" | "same-name">("decide");
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  // From when the modal appeared, not from the request's creation --
  // the event has no timestamp, and the two are milliseconds apart.
  const [deadline] = useState(() => Date.now() + DECISION_SECS * 1000);
  const secs = useCountdown(deadline);

  const answer = (approve: boolean, replaceExisting?: boolean) => {
    setBusy(true);
    setError(null);
    respond(request.request_id, approve, replaceExisting).then(
      () => onDone(),
      (e: unknown) => {
        setBusy(false);
        if (approve && replaceExisting === undefined && nameTaken(e)) {
          setStage("same-name");
        } else {
          setError(typeof e === "string" ? e : "Could not answer the request");
        }
      },
    );
  };

  // Auto-deny. The deny is sent rather than merely closing the modal:
  // the Rust side times out on its own at the same moment, but the
  // phone hears "no" sooner and the UI never shows a request the
  // desktop has forgotten. Its outcome is not shown -- a rejection here
  // means the Rust side got there first, which is the same result.
  // The ref guards against re-running: `respond` and `onDone` are new
  // closures each render, so this effect re-arms often.
  const timedOut = useRef(false);
  useEffect(() => {
    if (secs > 0 || timedOut.current) return;
    timedOut.current = true;
    respond(request.request_id, false).then(onDone, onDone);
  }, [secs, respond, request.request_id, onDone]);
  const locked = busy || secs === 0;

  return (
    <Dialog open onOpenChange={(o) => !o && !locked && answer(false)}>
      <DialogContent className="max-w-lg" showCloseButton={false}>
        {stage === "decide" ? (
          <>
            <DialogTitle>Pair {request.device_name}?</DialogTitle>
            <p className="mt-3 text-sm text-[#e6edf3]">
              A phone calling itself <strong>{request.device_name}</strong> is asking
              to pair. Its fingerprint should match what the phone shows:
            </p>
            <code className="mt-2 block break-words font-mono text-xs text-[#e6edf3]">
              {formatFingerprint(request.fingerprint)}
            </code>
            {/* Stated either way, not only when present: "no line" and
                "no key" would be indistinguishable, and the pairing
                record decides which signatures every later request has
                to carry. */}
            <p className="mt-2 text-xs text-[#8b949e]">
              {request.has_mldsa
                ? "Offered a post-quantum signing key (ML-DSA-65) as well as ECDSA."
                : "No post-quantum signing key offered; this phone will sign with ECDSA only."}
            </p>
            <p className="mt-2 text-xs text-[#8b949e]">
              Denied automatically in <span className="font-mono">{mmss(secs)}</span>
            </p>
          </>
        ) : (
          <>
            <DialogTitle>Replace the existing pairing for {request.device_name}?</DialogTitle>
            <p className="mt-3 text-sm text-[#e6edf3]">
              A phone with this name is already paired. Replacing it revokes the
              old one; keeping both leaves two devices with the same name.
            </p>
          </>
        )}
        {error ? (
          <p role="alert" className="mt-2 text-xs text-[#f85149]">
            {error}
          </p>
        ) : null}
        <div className="mt-5 flex justify-end gap-2">
          {stage === "decide" ? (
            <>
              <button
                type="button"
                disabled={locked}
                onClick={() => answer(false)}
                className="rounded border border-[#30363d] px-3 py-1.5 text-sm hover:bg-[#21262d] disabled:opacity-50"
              >
                Deny
              </button>
              <button
                type="button"
                disabled={locked}
                onClick={() => answer(true)}
                className="rounded bg-[#238636] px-3 py-1.5 text-sm font-medium text-white hover:bg-[#2ea043] disabled:opacity-50"
              >
                Approve
              </button>
            </>
          ) : (
            <>
              <button
                type="button"
                disabled={locked}
                onClick={() => setStage("decide")}
                className="rounded border border-[#30363d] px-3 py-1.5 text-sm hover:bg-[#21262d] disabled:opacity-50"
              >
                Cancel
              </button>
              <button
                type="button"
                disabled={locked}
                onClick={() => answer(true, false)}
                className="rounded border border-[#30363d] px-3 py-1.5 text-sm hover:bg-[#21262d] disabled:opacity-50"
              >
                Keep both
              </button>
              <button
                type="button"
                disabled={locked}
                onClick={() => answer(true, true)}
                className="rounded bg-[#238636] px-3 py-1.5 text-sm font-medium text-white hover:bg-[#2ea043] disabled:opacity-50"
              >
                Replace
              </button>
            </>
          )}
        </div>
      </DialogContent>
    </Dialog>
  );
}

/// Listens for `pairing-request` and shows one dialog at a time. Lives
/// at the app root rather than in Settings: the phone scans whenever it
/// likes, and a request that arrived with Settings closed must still be
/// answerable.
export function PairingRequestModal() {
  const { request, dismiss } = usePairingRequest();
  if (!request) return null;
  // Keyed so a second request gets a fresh countdown and a clean stage.
  return <PairingRequestDialog key={request.request_id} request={request} onDone={dismiss} />;
}
