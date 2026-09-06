import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import type { Transport } from "./transport";

/// The desktop transport: the webview talking to the Rust process it is
/// embedded in, through Tauri's own IPC.
///
/// This and `remote.ts` are the only modules that import
/// `@tauri-apps/api/core` or `@tauri-apps/api/event`. Tests that mock
/// those modules keep working unchanged because the calls still land
/// here.
export const local: Transport = {
  // Arity preserved on purpose: a command without arguments reaches
  // Tauri as `invoke(name)`, exactly as the wrappers called it before
  // the seam. Tests assert on that one-argument shape.
  call: (name, args) => (args === undefined ? invoke(name) : invoke(name, args)),
  listen: (event, cb) => listen(event, cb),
};
