import { defineConfig } from "vitest/config";

// Unit tests only. Playwright specs in `e2e/` use `@playwright/test`'s
// `test.describe` which is incompatible with vitest's runner; they're
// executed by `plz ui e2e`.
export default defineConfig({
  test: {
    include: ["src/**/*.test.ts"],
    exclude: ["node_modules", "dist", "e2e"],
  },
});
