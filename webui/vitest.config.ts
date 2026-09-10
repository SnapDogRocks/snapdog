import path from "node:path";
import { defineConfig } from "vitest/config";

export default defineConfig({
  resolve: {
    alias: {
      "@": path.resolve(import.meta.dirname, "./src"),
    },
  },
  test: {
    environment: "node",
    include: ["src/**/*.test.ts", "src/**/*.test.tsx"],
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
      ],
      // Starter floor: there is no test suite yet beyond a handful of
      // pure src/lib/ unit tests (format-time, utils, eq-response), set
      // just below today's actual measured coverage (~3.2% lines /
      // ~3.0% statements / ~1.1% functions / ~0.3% branches over the
      // whole src/ tree — see the PR description). Raise it as real
      // coverage grows; never lower it to make room for untested code.
      thresholds: {
        lines: 3,
        statements: 2.5,
        functions: 1,
        branches: 0.25,
      },
    },
  },
});
