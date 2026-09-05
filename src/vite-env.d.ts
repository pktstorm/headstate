/// <reference types="vite/client" />

interface ImportMetaEnv {
  /// Which transport `src/api/transport.ts` picks. Always present at
  /// runtime: `vite.config.ts` defines it to `desktop` when unset.
  readonly VITE_TARGET: "desktop" | "mobile";
}
