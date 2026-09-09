// From "vitest/config", not "vite": the `test` block below is a Vitest key,
// and Vite's own `defineConfig` rejects it.
import { defineConfig } from "vitest/config";
import { loadEnv } from "vite";
import react from "@vitejs/plugin-react";
import tailwindcss from "@tailwindcss/vite";

// `new URL` + `import.meta.url` resolves paths without needing
// `@types/node` for `node:path`/`__dirname`/`process.cwd()`.
const root = new URL(".", import.meta.url).pathname;

export default defineConfig(({ mode }) => ({
  plugins: [react(), tailwindcss()],
  resolve: { alias: { "@": new URL("./src", import.meta.url).pathname } },
  // Which transport `src/api/transport.ts` picks. Defined here rather
  // than defaulted at the use site so a build with the variable unset
  // is the desktop build, and so the value is a compile-time constant
  // the bundler can fold. `loadEnv` sees both `.env*` files and the
  // process environment, so `VITE_TARGET=mobile yarn build` works.
  define: {
    "import.meta.env.VITE_TARGET": JSON.stringify(
      loadEnv(mode, root, "VITE_").VITE_TARGET ?? "desktop",
    ),
  },
  // Tauri expects a fixed port and fails if it is taken.
  //
  // `host` comes from TAURI_DEV_HOST, which `tauri ios dev --host` sets
  // to the machine's public network address: a phone on the same LAN
  // cannot reach `localhost`, so a device run needs the dev server
  // bound to an address it can actually route to. Unset -- every
  // desktop run, and CI -- it stays on Vite's default loopback bind
  // rather than exposing the dev server on the network by accident.
  //
  // Read through `loadEnv` rather than `process.env`, for the same
  // reason as VITE_TARGET below: the project deliberately carries no
  // `@types/node`, and `loadEnv` sees the process environment anyway.
  // The prefix is the full variable name because `loadEnv` filters by
  // prefix and this one does not start with VITE_.
  server: {
    port: 1420,
    strictPort: true,
    host: loadEnv(mode, root, "TAURI_DEV_HOST").TAURI_DEV_HOST ?? false,
  },
  build: { target: "safari15", sourcemap: true },
  test: {
    // Vitest's default glob walks the whole tree, and this project keeps
    // git worktrees at `.worktrees/<branch>/`. A bare `vitest run` then
    // collected every sibling branch's tests too -- 4837 tests across 482
    // files instead of this checkout's ~500 -- and stale branches fail
    // against current code, so `make test-ui` disagreed with CI (which
    // has no worktrees) and the local signal was worthless.
    //
    // Anchoring `include` to `src/` is the whole fix; an explicit
    // `.worktrees` exclude was measured to change nothing and left out.
    include: ["src/**/*.{test,spec}.{ts,tsx}"],
    environment: "jsdom",
    globals: true,
    // 15s, not vitest's default 5s.
    //
    // Not because any test is slow: 58% of this suite's wall-clock is
    // jsdom CONSTRUCTION -- 141 environments, one per test file, about
    // 154s of the 98s run (they overlap across workers). The default
    // budget is measured against a test whose own work is a fraction of
    // what surrounds it.
    //
    // A fully synchronous test in WorktreesPage.test.tsx -- render, then
    // assert, no await anywhere -- hit "Test timed out in 5000ms" on a
    // loaded Windows runner and blocked the v5.5.0 release, while 1548
    // of 1549 tests passed and a re-run of the same commit went green.
    // The whole 88-test file takes 2911ms when the runner is healthy:
    // inside the budget, with no margin, and the two runs of that one
    // commit differed by 1.5x on their own.
    //
    // The cost of raising it is that a genuinely hung test reports after
    // 15s instead of 5s. The cost of leaving it is a release blocked by
    // a test unrelated to the change being released. See #677, which
    // also tracks reducing the jsdom cost itself -- the actual wound,
    // of which this timeout is only the bleeding.
    testTimeout: 15_000,
    coverage: { provider: "v8", reporter: ["text", "json-summary"] },
  },
}));
