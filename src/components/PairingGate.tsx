import { type ReactNode, useEffect } from "react";
import { useConnectionState } from "@/api/connection";
import { IS_MOBILE_BUILD } from "@/lib/target";
import { dismissSplash } from "../splash";
import { PairingScreen } from "./PairingScreen";

/// The phone's gate: pairing, not GitHub auth.
///
/// On the desktop this renders its children and nothing else -- there is
/// no pairing to do, and `AuthGate` below is the right gate there.
///
/// On the phone it is the outermost gate, ABOVE `AuthGate`. That order
/// is the whole point. `AuthGate` gates on `get_auth_state`, and on the
/// mobile build every desktop command is forwarded over `remote_call`,
/// which rejects with "not paired with a desktop" while unpaired. So a
/// freshly installed phone failed the auth check and landed on the
/// desktop's remediation screen: "Headstate needs the GitHub CLI", with
/// `brew install gh` and `gh auth login`. A phone has no Homebrew, no
/// shell and -- by design -- no GitHub token, so the instructions were
/// impossible and there was no way past the screen. Pairing was
/// unreachable, which made the whole app unreachable.
///
/// Guarded on `IS_MOBILE_BUILD` rather than `useIsMobile()`: this is a
/// capability question, not a layout one. A desktop window dragged
/// narrower is still a desktop, still has `gh`, and must still get
/// `AuthGate`.
///
/// Whether the DESKTOP's GitHub auth is healthy is a property of the
/// paired desktop. If it is ever surfaced on the phone it belongs in the
/// connection banner, worded as being about the desktop -- never as
/// `brew install gh` addressed to a phone.
export function PairingGate({ children }: { children: ReactNode }) {
  if (!IS_MOBILE_BUILD) return <>{children}</>;
  return <MobilePairingGate>{children}</MobilePairingGate>;
}

/// Split out so the desktop build's branch above is a plain early
/// return with no hooks behind it. `IS_MOBILE_BUILD` is a compile-time
/// constant, so exactly one of these two paths exists in a given bundle
/// and the hook order is stable within it.
function MobilePairingGate({ children }: { children: ReactNode }) {
  const state = useConnectionState();

  // The splash must come down on EVERY terminal state, not just the
  // happy one. This is the v1.0.0 hang generalised: the splash is a
  // fixed inset-0 z-index-9999 overlay, so any branch that renders a
  // real screen without dismissing it hides that screen forever. Here
  // "unpaired" is a perfectly good screen, and the only state that
  // should still hold the splash is "we do not know yet".
  //
  // `dismissSplash` is idempotent and honours its own minimum-visible
  // floor, so calling it on each settled render is safe.
  const settled = state.kind !== "unknown";
  useEffect(() => {
    if (settled) dismissSplash();
  }, [settled]);

  // `unknown` is the phone before `connection_state` has answered.
  // Rendering nothing keeps the splash up for the few milliseconds that
  // takes, rather than flashing the pairing screen at someone who is
  // already paired.
  if (state.kind === "unknown") return null;

  // Unpaired is the first-run case. Revoked is the same screen for a
  // different reason: the desktop has withdrawn this phone's access, the
  // banner's own text says "pair again", and this is where pairing
  // happens. Sending revoked here is what makes that instruction true --
  // it used to name an action the app did not offer.
  //
  // Revoked carries the desktop's name through, because the screen has
  // to SAY why it is asking again. Landing on a bare "Pair with your
  // desktop" after being revoked reads as the app having forgotten,
  // rather than as the desktop having decided -- and the walkthrough
  // (step 6.1) requires the message.
  if (state.kind === "revoked") return <PairingScreen revokedBy={state.desktop} />;
  if (state.kind === "unpaired") return <PairingScreen />;

  return <>{children}</>;
}
