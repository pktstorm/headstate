import { useState } from "react";
import { useUnpair } from "@/api/pairing";
import { type ConnectionState, useConnectionState } from "@/api/connection";
import { Button } from "./ui/button";

/// The phone's half of the Phone settings section.
///
/// The desktop's half -- "Allow phone connections", the QR generator,
/// and the paired-devices list -- is what used to render here on BOTH
/// builds, because `SettingsDialog` had no build guard at all. On a
/// phone that panel asks the phone to BE a desktop: a checkbox for a
/// listener it does not run, a QR to be scanned by some other phone, and
/// a device list whose commands (`get_remote_enabled`,
/// `issue_pairing_token`, `list_paired_devices`) are every one of them
/// `Class::Local` and refused before they reach the wire. All three were
/// inert, and one of them was where the "Not paired · tap to pair"
/// banner sent people.
///
/// This is what a phone actually needs: which desktop it drives, and the
/// way back out.

function describe(state: ConnectionState): { desktop: string; status: string } | null {
  switch (state.kind) {
    case "local":
    case "unknown":
    case "unpaired":
      return null;
    case "connected":
      return { desktop: state.desktop, status: "Connected" };
    case "connecting":
      return { desktop: state.desktop, status: "Connecting…" };
    case "unreachable":
      return { desktop: state.desktop, status: "Unreachable" };
    case "revoked":
      return { desktop: state.desktop, status: "This phone was removed" };
  }
}

export function PairedDesktopPanel() {
  const state = useConnectionState();
  const unpair = useUnpair();
  const [confirming, setConfirming] = useState(false);
  const paired = describe(state);

  if (paired === null) {
    // Unpaired is not reachable from here in practice -- `PairingGate`
    // owns the whole screen in that state -- but Settings is mounted
    // from the connection banner, and a desktop revoking the phone
    // while the dialog is open would otherwise leave this section
    // rendering a desktop that is no longer there.
    return (
      <div className="mt-5 flex flex-col gap-2">
        <span className="text-sm font-medium">Desktop</span>
        <p className="text-xs text-[#8b949e]">This phone is not paired with a desktop.</p>
      </div>
    );
  }

  return (
    <div className="mt-5 flex flex-col gap-2">
      <span className="text-sm font-medium">Desktop</span>
      <p className="text-sm">{paired.desktop}</p>
      <p className="text-xs text-[#8b949e]">{paired.status}</p>

      {confirming ? (
        <div className="mt-2 space-y-2 rounded border border-[#f85149]/30 bg-[#f85149]/10 p-3">
          <p className="text-sm text-[#e6edf3]">
            Forget {paired.desktop}? This phone will stop showing its pull requests, and
            everything cached from it is cleared. You can pair again by scanning a new
            code on that desktop.
          </p>
          {unpair.isError ? (
            <p role="alert" className="text-xs text-[#f85149]">
              {unpair.error instanceof Error ? unpair.error.message : String(unpair.error)}
            </p>
          ) : null}
          <div className="flex gap-2">
            <Button
              variant="destructive"
              disabled={unpair.isPending}
              onClick={() => unpair.mutate()}
              className="min-h-11 flex-1 text-base"
            >
              {unpair.isPending ? "Forgetting…" : "Forget desktop"}
            </Button>
            <Button
              variant="ghost"
              disabled={unpair.isPending}
              onClick={() => setConfirming(false)}
              className="min-h-11 flex-1 text-base"
            >
              Cancel
            </Button>
          </div>
        </div>
      ) : (
        <Button
          variant="outline"
          onClick={() => setConfirming(true)}
          className="mt-2 min-h-11 w-full text-base"
        >
          Forget desktop
        </Button>
      )}
      <p className="text-xs text-[#8b949e]">
        Your desktop holds the GitHub sign-in and does the work; this phone only asks it
        to.
      </p>
    </div>
  );
}
