import js from "@eslint/js";
import globals from "globals";
import reactHooks from "eslint-plugin-react-hooks";
import tseslint from "typescript-eslint";

export default tseslint.config(
  { ignores: ["ui/dist", "ui/src/bindings.ts"] },
  {
    files: ["ui/**/*.{ts,tsx}"],
    // Type-aware: a command whose failure nobody handles is a lint error.
    extends: [
      js.configs.recommended,
      ...tseslint.configs.recommendedTypeChecked,
    ],
    languageOptions: {
      ecmaVersion: 2022,
      globals: globals.browser,
      parserOptions: {
        projectService: true,
        tsconfigRootDir: import.meta.dirname,
      },
    },
    plugins: { "react-hooks": reactHooks },
    rules: {
      ...reactHooks.configs.recommended.rules,
      // `void promise` is not handling it: start fire-and-forget work with
      // `fire` / `runCommand` (notify.ts) so failures reach the person.
      "@typescript-eslint/no-floating-promises": [
        "error",
        { ignoreVoid: false },
      ],
      "@typescript-eslint/no-unused-vars": [
        "error",
        { argsIgnorePattern: "^_" },
      ],
    },
  },
  {
    // `await act(async () => …)` flushes React work even when the callback
    // itself has nothing to await.
    files: ["ui/**/*.test.{ts,tsx}"],
    rules: { "@typescript-eslint/require-await": "off" },
  },
  {
    files: ["ui/e2e/**/*.mjs", "scripts/**/*.mjs"],
    extends: [js.configs.recommended],
    languageOptions: {
      ecmaVersion: 2022,
      globals: { ...globals.node, ...globals.browser },
    },
  },
);
