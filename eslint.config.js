import js from "@eslint/js";
import globals from "globals";
import tseslint from "typescript-eslint";
import reactHooks from "eslint-plugin-react-hooks";
import reactRefresh from "eslint-plugin-react-refresh";

export default tseslint.config(
  // Both crates' build directories: an iOS build leaves tauri-codegen's
  // compressed `.js` assets under src-mobile/target, which eslint
  // otherwise tries to parse.
  {
    ignores: [
      "dist",
      "src-tauri/target",
      "src-mobile/target",
      "src-mobile/gen/apple/build",
      "coverage",
      ".remember",
    ],
  },
  {
    extends: [js.configs.recommended, ...tseslint.configs.recommended],
    files: ["**/*.{ts,tsx}"],
    languageOptions: { ecmaVersion: 2022, globals: globals.browser },
    plugins: { "react-hooks": reactHooks, "react-refresh": reactRefresh },
    rules: {
      ...reactHooks.configs.recommended.rules,
      "react-refresh/only-export-components": ["warn", { allowConstantExport: true }],
    },
  },
);
