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
  server: { port: 1420, strictPort: true },
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
    coverage: { provider: "v8", reporter: ["text", "json-summary"] },
  },
}));
