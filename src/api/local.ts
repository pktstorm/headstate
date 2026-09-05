import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import type { Transport } from "./transport";

/// The desktop transport: the webview talking to the Rust process it is
/// embedded in, through Tauri's own IPC.
///
/// This is the ONLY module that imports `@tauri-apps/api/core` or
/// `@tauri-apps/api/event`. Tests that mock those modules keep working
/// unchanged because the calls still land here.
export const local: Transport = {
  call: (name, args) => invoke(name, args),
  listen: (event, cb) => listen(event, cb),
};
