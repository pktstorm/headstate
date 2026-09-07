/// Which app this bundle IS, as opposed to how wide its window happens
/// to be.
///
/// This is the capability question: the phone has no `gh` CLI, no GitHub
/// token, no repo checkouts, no filesystem and no Docker of its own.
/// Everything it does is a command forwarded to the paired desktop, and
/// the commands the desktop refuses to forward (`Class::Local` in
/// `src-tauri/src/remote/surface.rs`) can never work there however the
/// window is sized.
///
/// It is deliberately NOT `useIsMobile()`. That hook answers "render the
/// phone layout" and is true for the mobile build *or* any viewport
/// under `MOBILE_BREAKPOINT` -- and a narrow desktop window has `gh`, a
/// filesystem and an updater. Using it to decide capability silently
/// breaks the desktop at 767px wide, which is why the two are separate
/// names rather than one hook with a comment.
///
/// The rule of thumb: if the answer would change when the user drags
/// their desktop window narrower, use `useIsMobile()`. If it would not,
/// use this.
///
/// `VITE_TARGET` is a Vite `define` (see `vite.config.ts`), so this folds
/// to a literal at build time and the branch it guards is dropped from
/// the bundle it does not belong in. That also means it is a plain
/// constant, not a hook: it cannot change while the app is running, and
/// nothing should re-render because of it.
export const IS_MOBILE_BUILD = import.meta.env.VITE_TARGET === "mobile";

/// The inverse, named so a desktop-only branch reads as one at the call
/// site instead of as a negation of the phone.
export const IS_DESKTOP_BUILD = !IS_MOBILE_BUILD;
