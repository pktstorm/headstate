import type { UnlistenFn } from "./transport";

/// Call a Tauri `unlisten` without letting a teardown race become an error.
///
/// React 19 StrictMode double-mounts effects on purpose. When the first
/// mount is torn down before its `listen()` promise resolves, the cleanup
/// runs `unlisten` against a listener Tauri has ALREADY removed, and its
/// internal `listeners[eventId]` lookup is undefined:
///
///   TypeError: undefined is not an object
///     (evaluating 'listeners[eventId].handlerId')
///
/// Three of those were logged on every single `make dev` startup. Nothing
/// leaked -- the listener is gone, and it is the teardown that fails --
/// but it was three screens of stack trace during exactly the debugging
/// sessions where the console matters, and it had to be ruled out before
/// it could be dismissed.
///
/// Swallowing is right HERE and nowhere else: the only thing this call can
/// achieve is removing a listener, so a failure means the listener is
/// already gone -- the desired end state. There is no partial outcome to
/// report and nothing a caller could do differently.
///
/// THE SUBSCRIBE SIDE, since the bare `() => {}` rejection handlers beside
/// every `listen(...).then(...)` in `hooks.ts` point here. A `listen()` that
/// REJECTS (IPC transport gone, webview torn down mid-call) is the mirror
/// of the above: no listener was created, which is the state the effect's
/// cleanup wants anyway, and there is nothing a caller could do
/// differently -- so it is swallowed for the same reason. What is not
/// acceptable is leaving it unhandled, because an unhandled rejection is
/// indistinguishable from a bug at the console, which is the whole
/// complaint this file opens with. Eleven of the seventeen such sites had
/// no rejection handler at all until
/// `@typescript-eslint/no-floating-promises` was measured and turned on
/// (#892); the other six already had one, which is why the fix was to
/// match the existing shape rather than invent a helper.
///
/// The `catch` on the return value is not redundant with the try/catch.
/// Tauri's `unlisten` is sync in its current shape, but the observed dev
/// error arrived as an UNHANDLED REJECTION, so the failure crossed a
/// promise boundary. A version that returns a promise would sail straight
/// past a bare try/catch and reproduce the exact bug this exists to fix.
export function safeUnlisten(fn: UnlistenFn | undefined): void {
  if (!fn) return;
  try {
    const returned = fn() as unknown;
    if (returned instanceof Promise) returned.catch(() => {});
  } catch {
    // Deliberately empty: see above. The listener is gone either way.
  }
}
