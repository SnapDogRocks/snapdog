import path from "node:path";
import { defineConfig } from "vitest/config";

export default defineConfig({
  resolve: {
    alias: {
      "@": path.resolve(import.meta.dirname, "./src"),
    },
  },
  test: {
    // Pure modules run in Node; components (`*.test.tsx`) in a DOM.
    projects: [
      {
        extends: true,
        test: { name: "unit", environment: "node", include: ["src/**/*.test.ts"] },
      },
      {
        extends: true,
        test: {
          name: "components",
          environment: "jsdom",
          include: ["src/**/*.test.tsx"],
          setupFiles: ["src/test/setup-dom.ts"],
        },
      },
    ],
    coverage: {
      provider: "v8",
      reporter: ["text", "html", "json-summary"],
      // Whole source tree, not just the tested files — an untested
      // component genuinely drags the number down. That's the point:
      // the floor should track real coverage, not a flattering subset.
      include: ["src/**/*.ts", "src/**/*.tsx"],
      exclude: [
        "src/**/*.d.ts",
        "src/**/*.test.ts",
        "src/**/*.test.tsx",
        "src/test/**",
      ],
      // Floor set about a point below the measured coverage with the component tests
      // (~25.4% lines / ~24.2% statements / ~19.1% functions / ~19.0% branches over
      // the whole src/ tree). Raise it as coverage grows; never lower it to make
      // room for untested code.
      thresholds: {
        lines: 24,
        statements: 23,
        functions: 18,
        branches: 18,
      },
    },
  },
});
