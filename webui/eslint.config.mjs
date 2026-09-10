import { defineConfig, globalIgnores } from "eslint/config";
import nextVitals from "eslint-config-next/core-web-vitals";
import nextTs from "eslint-config-next/typescript";
import tseslint from "typescript-eslint";

const eslintConfig = defineConfig([
  ...nextVitals,
  ...nextTs,
  // eslint-config-next/typescript only applies typescript-eslint's basic
  // `recommended` preset (not type-aware) plus two rules. Layer the
  // type-aware strict + stylistic presets on top, scoped to TS files only —
  // without a `files` filter they'd also apply to this .mjs config file,
  // which is in no tsconfig and would fail with "was not found by the
  // project service".
  ...tseslint.configs.strictTypeChecked.map((c) => ({ ...c, files: ["**/*.ts", "**/*.tsx"] })),
  ...tseslint.configs.stylisticTypeChecked.map((c) => ({ ...c, files: ["**/*.ts", "**/*.tsx"] })),
  {
    files: ["**/*.ts", "**/*.tsx"],
    languageOptions: {
      parserOptions: {
        // Replaces a manual `project` list and also covers files that are
        // in no tsconfig.
        projectService: true,
        tsconfigRootDir: import.meta.dirname,
      },
    },
  },
  {
    files: ["**/*.ts", "**/*.tsx"],
    rules: {
      "@next/next/no-img-element": "off",
      // This is a UI codebase that formats numeric values (ms, Hz, dB, px,
      // percentages) into template strings throughout; that's always
      // intentional and safe. Keep the rest of the type-aware check (it
      // still flags objects, `any`, etc. as before).
      "@typescript-eslint/restrict-template-expressions": ["error", { allowNumber: true }],
    },
  },
  {
    // Relaxations exclusively for tests, never globally.
    files: ["**/*.test.ts", "**/*.test.tsx", "**/*.spec.ts", "**/tests/**/*.ts"],
    rules: {
      "@typescript-eslint/no-explicit-any": "off",
      "@typescript-eslint/no-unsafe-assignment": "off",
      "@typescript-eslint/no-unsafe-member-access": "off",
      "@typescript-eslint/no-non-null-assertion": "off",
    },
  },
  // Override default ignores of eslint-config-next.
  globalIgnores([
    // Default ignores of eslint-config-next:
    ".next/**",
    "out/**",
    "build/**",
    "next-env.d.ts",
  ]),
]);

export default eslintConfig;
