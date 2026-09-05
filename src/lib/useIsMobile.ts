import { useSyncExternalStore } from "react";

/// Below this width the layout is the phone one. Chosen to match the
/// Tailwind `md` breakpoint so the two never disagree about what
/// "narrow" means.
export const MOBILE_BREAKPOINT = 768;

const QUERY = `(max-width: ${MOBILE_BREAKPOINT - 1}px)`;

/// `matchMedia` is absent under SSR and under jsdom. Absent means "not
/// narrow" rather than a throw, so every existing component test keeps
/// rendering the desktop layout it was written against.
function media(): MediaQueryList | null {
  if (typeof window === "undefined" || typeof window.matchMedia !== "function") return null;
  return window.matchMedia(QUERY);
}

function subscribe(onChange: () => void): () => void {
  const mql = media();
  if (mql === null) return () => {};
  mql.addEventListener("change", onChange);
  return () => mql.removeEventListener("change", onChange);
}

function snapshot(): boolean {
  return media()?.matches ?? false;
}

function serverSnapshot(): boolean {
  return false;
}

/// Whether to render the phone layout.
///
/// True on the mobile build unconditionally -- the companion app never
/// shows a desktop layout, whatever the webview reports -- and true on
/// any build whose viewport is narrower than `MOBILE_BREAKPOINT`. The
/// second half is what makes the mobile layout reachable from a plain
/// browser during development: the desktop window enforces a minimum
/// width of 1000px, so it never fires there.
///
/// Components read this and switch their layout in place. There is no
/// second component tree for the phone; a component that forks would
/// drift from its desktop twin the first time one of them is touched.
export function useIsMobile(): boolean {
  const narrow = useSyncExternalStore(subscribe, snapshot, serverSnapshot);
  return import.meta.env.VITE_TARGET === "mobile" || narrow;
}
