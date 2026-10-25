import { defineConfig } from "@playwright/test";

// E2E tests run against the Vite dev server. CI builds wasm via the
// cfasimWasm plugin on first request, so the first navigation has a
// generous timeout; subsequent tests reuse the cached module.
export default defineConfig({
  testDir: "./e2e",
  fullyParallel: true,
  forbidOnly: !!process.env.CI,
  workers: process.env.CI ? 2 : undefined,
  reporter: process.env.CI ? "github" : "list",
  use: {
    baseURL: `http://localhost:${process.env.E2E_PORT}`,
    trace: "retain-on-failure",
  },
  webServer: {
    command: "pnpm exec vite --port 0",
    stdout: "pipe",
    env: { ...process.env, NO_COLOR: "1" } as Record<string, string>,
    wait: { stdout: /localhost:(?<e2e_port>\d+)/ },
    reuseExistingServer: false,
    timeout: 120_000,
  },
  projects: [{ name: "chromium", use: { browserName: "chromium" } }],
});
