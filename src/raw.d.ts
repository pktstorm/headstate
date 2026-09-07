/// `?raw` imports, so the surface guard can read `surface.rs` and
/// `tauri.ts` as text without `@types/node`. Vite resolves the suffix at
/// transform time; TypeScript needs to be told the result is a string.
declare module "*?raw" {
  const content: string;
  export default content;
}
