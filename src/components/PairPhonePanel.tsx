import { useState } from "react";
import { QRCodeSVG } from "qrcode.react";
import { useIssuePairingToken } from "../api/hooks";
import type { PairingQrPayload } from "../api/tauri";
import { mmss, useCountdown } from "@/lib/countdown";
import { formatFingerprint } from "@/lib/fingerprint";

const button =
  "rounded border border-[#30363d] px-3 py-1 text-sm text-[#e6edf3] hover:bg-[#21262d] disabled:opacity-50";

/// Settings > Phone > **Pair a phone**.
///
/// Asks the Rust side for a token and hands the payload to
/// [`PairingCode`], keyed on the token so Regenerate mounts a fresh
/// countdown rather than adjusting a running one.
export function PairPhonePanel() {
  const issue = useIssuePairingToken();
  const [payload, setPayload] = useState<PairingQrPayload | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);

  const generate = () => {
    setBusy(true);
    setError(null);
    issue()
      .then(setPayload, (e: unknown) =>
        setError(typeof e === "string" ? e : "Could not start pairing"),
      )
      .finally(() => setBusy(false));
  };

  return (
    <div className="mt-5 flex flex-col gap-2">
      <span className="text-sm font-medium">Pair a phone</span>
      {payload === null ? (
        <>
          <p className="text-xs text-[#8b949e]">
            Shows a code to scan with the Headstate companion app. Turns on
            phone connections if they are off.
          </p>
          <button type="button" disabled={busy} onClick={generate} className={`self-start ${button}`}>
            {busy ? "Starting…" : "Pair a phone"}
          </button>
        </>
      ) : (
        <PairingCode
          key={payload.token}
          payload={payload}
          busy={busy}
          onRegenerate={generate}
          onHide={() => setPayload(null)}
        />
      )}
      {error ? (
        <p role="alert" className="text-xs text-[#f85149]">
          {error}
        </p>
      ) : null}
    </div>
  );
}

/// The QR code with its countdown. The countdown is read from the
/// payload's `exp` rather than started here: the token expires on the
/// Rust side's clock, and a countdown that ran from "when the QR
/// appeared" would drift from it by however long the command took.
///
/// The fingerprint is shown BESIDE the QR, in the same groups the phone
/// shows, because the QR carries it and a phone that scanned a
/// different code (a stale screenshot, a look-alike) would show a
/// different string. That comparison is the only defence against a
/// wrong `fp`, so it has to be easy to make.
function PairingCode({
  payload,
  busy,
  onRegenerate,
  onHide,
}: {
  payload: PairingQrPayload;
  busy: boolean;
  onRegenerate: () => void;
  onHide: () => void;
}) {
  const secs = useCountdown(payload.exp * 1000);
  const expired = secs === 0;

  return (
    <div className="flex items-start gap-5">
      {/* White quiet zone on purpose: a QR on the app's dark background
          scans unreliably, and this one is scanned by a phone camera at
          arm's length. Blanked on expiry rather than greyed -- a
          scannable-looking code that the desktop will refuse is the
          confusing outcome. */}
      {expired ? (
        <div
          aria-hidden="true"
          className="flex h-48 w-48 shrink-0 items-center justify-center rounded bg-[#161b22] text-xs text-[#8b949e]"
        >
          Expired
        </div>
      ) : (
        <div role="img" aria-label="Pairing QR code" className="shrink-0 rounded bg-white p-2">
          <QRCodeSVG value={JSON.stringify(payload)} size={176} level="M" />
        </div>
      )}
      <div className="flex min-w-0 flex-col gap-2 text-sm">
        <p className="text-xs text-[#8b949e]">
          Scan with the Headstate companion app. The phone will show this
          fingerprint; approve the pairing only if the two match.
        </p>
        <span className="text-xs font-medium text-[#8b949e]">Fingerprint (SHA256)</span>
        <code className="break-words font-mono text-xs text-[#e6edf3]">
          {formatFingerprint(payload.fp)}
        </code>
        {expired ? (
          <p className="text-xs text-[#d29922]">
            This code has expired. Generate a new one to try again.
          </p>
        ) : (
          <p className="text-xs text-[#8b949e]">
            Expires in <span className="font-mono">{mmss(secs)}</span>
          </p>
        )}
        <div className="flex gap-2">
          <button type="button" disabled={busy} onClick={onRegenerate} className={button}>
            {busy ? "Starting…" : "Regenerate"}
          </button>
          <button
            type="button"
            onClick={onHide}
            className="rounded px-3 py-1 text-sm text-[#8b949e] hover:bg-[#21262d]"
          >
            Hide
          </button>
        </div>
      </div>
    </div>
  );
}
