import { useState } from "react";
import { type ConnectionState, useConnectionState } from "@/api/connection";
import { useIsMobile } from "@/lib/useIsMobile";
import { relativeTime } from "@/lib/time";
import { SettingsDialog } from "./SettingsDialog";

/// One line of text per state, and the dot colour beside it.
///
/// Every line names the desktop where there is one: "unreachable" on
/// its own does not say what is unreachable, and a phone paired with
/// two desktops later will need the name to tell them apart.
function describeState(state: Exclude<ConnectionState, { kind: "local" }>): {
  text: string;
  dot: string;
} {
  switch (state.kind) {
    case "connected":
      return {
        text: `${state.desktop} · reachable · last poll ${
          state.lastPoll ? relativeTime(state.lastPoll) : "not yet"
        }`,
        dot: "bg-[#3fb950]",
      };
    case "connecting":
      return { text: `Connecting to ${state.desktop}…`, dot: "bg-[#58a6ff]" };
    case "unreachable":
      return {
        text: `${state.desktop} is unreachable · last seen ${
          state.lastPoll ? relativeTime(state.lastPoll) : "never"
        }`,
        dot: "bg-[#d29922]",
      };
    case "revoked":
      return {
        text: `${state.desktop} revoked this phone · pair again`,
        dot: "bg-[#f85149]",
      };
    case "unpaired":
      return { text: "Not paired with a desktop · tap to pair", dot: "bg-[#8b949e]" };
    case "unknown":
      return { text: "Desktop connection status unavailable", dot: "bg-[#8b949e]" };
  }
}

/// The strip at the top of every phone screen: which desktop this
/// phone drives, whether it can be reached, and when it last heard
/// from GitHub.
///
/// Renders nothing on the desktop, where the app IS the desktop and
/// there is no connection to describe. Tapping opens Settings on the
/// Phone topic, which is where pairing lives.
export function ConnectionBanner() {
  const isMobile = useIsMobile();
  const state = useConnectionState();
  const [settingsOpen, setSettingsOpen] = useState(false);
  if (!isMobile || state.kind === "local") return null;
  const { text, dot } = describeState(state);
  return (
    <>
      <button
        type="button"
        onClick={() => setSettingsOpen(true)}
        title="Pairing settings"
        className="flex w-full shrink-0 items-center gap-2 border-b border-[#30363d] bg-[#161b22] px-4 py-2 text-left text-xs text-[#e6edf3] hover:bg-[#21262d]"
      >
        <span className={`h-2 w-2 shrink-0 rounded-full ${dot}`} aria-hidden="true" />
        <span className="min-w-0 flex-1 truncate">{text}</span>
        <span className="shrink-0 text-[#8b949e]">Pairing</span>
      </button>
      {/* Mounted only while open: Settings reads half a dozen
          preference queries, and the banner is on every screen. */}
      {settingsOpen ? (
        <SettingsDialog open onOpenChange={setSettingsOpen} initialSection="phone" />
      ) : null}
    </>
  );
}
