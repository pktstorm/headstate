import { useQuery } from "@tanstack/react-query";
import { invoke } from "@tauri-apps/api/core";

/// What the mobile crate's `connection_state` command answers with.
///
/// The command belongs to issue #514 (the mobile client commands) and
/// does not exist yet; this file is the frontend's side of that
/// contract, written first so the connection banner has something to
/// render against. `state` is the five-way enum from the design spec;
/// `desktop` is the paired desktop's name from the pairing QR, null
/// while unpaired; `last_poll` is when the desktop last reported a
/// successful GitHub poll over `/v1/events`, ISO 8601, null before the
/// first one.
interface ConnectionReport {
  state: "unpaired" | "connecting" | "connected" | "unreachable" | "revoked";
  desktop: string | null;
  last_poll: string | null;
}

/// The connection as the UI sees it.
///
/// `local` is the desktop app talking to its own Rust process: there is
/// no connection to report, and the banner renders nothing. `unknown` is
/// the mobile build before `connection_state` has answered -- including
/// when the command is not there, which is the case until #514 lands.
/// The rest are the command's own states, with the desktop's name
/// attached because every banner line needs it.
export type ConnectionState =
  | { kind: "local" }
  | { kind: "unknown" }
  | { kind: "unpaired" }
  | {
      kind: "connecting" | "connected" | "unreachable" | "revoked";
      desktop: string;
      lastPoll: string | null;
    };

/// How often the banner re-asks. Five seconds is fast enough that a
/// desktop going away is noticed before the user acts on stale data,
/// and the call is a local IPC round trip, not a network request.
const CONNECTION_POLL_MS = 5_000;

/// The mobile crate's answer, or a rejection where the command does
/// not exist. Routed through `invoke` directly for now; once the
/// transport seam (#527) is on main this becomes `transport.call`, so
/// that nothing outside `src/api` ever imports from `@tauri-apps/api`.
function connectionState(): Promise<ConnectionReport> {
  return invoke<ConnectionReport>("connection_state");
}

function fromReport(report: ConnectionReport): ConnectionState {
  if (report.state === "unpaired") return { kind: "unpaired" };
  return {
    kind: report.state,
    // A paired desktop always has a name -- it came from the QR -- but
    // the wire type allows null, and a banner reading "null is
    // unreachable" is worse than a generic noun.
    desktop: report.desktop ?? "Desktop",
    lastPoll: report.last_poll,
  };
}

const LOCAL: ConnectionState = { kind: "local" };

/// The desktop's answer: a constant. No query, and therefore no
/// QueryClient needed -- the banner mounts in every window and must
/// not make the desktop pay for a question only the phone asks.
function useLocalConnectionState(): ConnectionState {
  return LOCAL;
}

/// The phone's answer: poll `connection_state` and report `unknown`
/// until it answers, which it will not do until #514 fills the command
/// in -- the banner says so rather than crashing.
function useRemoteConnectionState(): ConnectionState {
  const { data } = useQuery({
    queryKey: ["connection-state"],
    queryFn: connectionState,
    refetchInterval: CONNECTION_POLL_MS,
    // A missing command rejects every time; retrying three times with
    // backoff before the next interval only delays the same answer.
    retry: false,
  });
  return data === undefined ? { kind: "unknown" } : fromReport(data);
}

/// The current connection to the paired desktop.
///
/// Chosen once, at module load, from the build target: the target is a
/// compile-time constant, so this is one hook or the other for the life
/// of the process rather than a hook called conditionally.
export const useConnectionState: () => ConnectionState =
  import.meta.env.VITE_TARGET === "mobile" ? useRemoteConnectionState : useLocalConnectionState;
