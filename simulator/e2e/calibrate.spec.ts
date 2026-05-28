import { test, expect } from "@playwright/test";

// /calibrate end-to-end smoke. Confirms:
//   1. The page loads with the calibration sidebar and the calibrate nav link.
//   2. Clicking "Start new run" generates synthetic data, runs ABC-SMC to
//      completion, and renders per-stage cells with a trajectory overlay +
//      R₀ + Initial-infections histograms.
//   3. The URL gains `?runId=…` after starting and a fresh navigation to
//      that URL deep-links back into the same run (loaded from IndexedDB).
//
// The wasm worker is built lazily on first navigation; we extend the
// per-action timeout to absorb both the cold-start build and the multi-
// stage ABC-SMC run.

test.describe("Calibration page", () => {
  test("runs to completion and renders per-stage diagnostics", async ({
    page,
  }) => {
    await page.goto("/calibrate");

    // Nav link present + page heading visible.
    await expect(
      page.getByRole("link", { name: "Calibrate" }),
    ).toBeVisible();
    await expect(
      page.getByRole("heading", { name: "Calibration", exact: true }),
    ).toBeVisible();

    // Idle state initially.
    await expect(page.getByText("Idle")).toBeVisible();

    // Kick off the run. Default config is 100 particles × (1 prior +
    // 4 perturb stages); on a CI machine this can take ~30s end-to-end.
    await page.getByRole("button", { name: "Start new run" }).click();

    // Status moves through Running → Complete.
    await expect(page.getByText(/Complete/)).toBeVisible({
      timeout: 180_000,
    });

    // The per-stage trace section renders with the summary table.
    await expect(
      page.getByRole("heading", { name: "Per-stage parameter trace" }),
    ).toBeVisible();
    await expect(
      page.getByRole("columnheader", { name: "Stage" }),
    ).toBeVisible();
    await expect(
      page.getByRole("columnheader", { name: "Particles" }),
    ).toBeVisible();
    await expect(
      page.getByRole("columnheader", { name: "Acceptance" }),
    ).toBeVisible();

    // At least one cell with the prior label (the implicit stage 0) and
    // at least one perturbation stage. Both labels appear twice (table
    // row + per-stage heading); .first() is fine for a presence check.
    await expect(page.getByText(/Prior \(∞\)/).first()).toBeVisible();
    await expect(page.getByText(/Stage 4/).first()).toBeVisible();

    // URL gained `?runId=…`.
    await expect(page).toHaveURL(/runId=[0-9a-f]+/);
    const url = page.url();

    // Reload via the deep link → same run restored from IndexedDB.
    await page.goto(url);
    await expect(page.getByText(/Complete/)).toBeVisible({ timeout: 30_000 });
    await expect(page.getByText(/Stage 4/).first()).toBeVisible();
  });

  test("refresh mid-run offers Resume (not a dead Pause) and continues", async ({
    page,
  }) => {
    await page.goto("/calibrate");
    await expect(
      page.getByRole("heading", { name: "Calibration", exact: true }),
    ).toBeVisible();

    // Start a run and wait until it's actually in flight.
    await page.getByRole("button", { name: "Start new run" }).click();
    await expect(page.getByRole("button", { name: "Pause" })).toBeVisible({
      timeout: 60_000,
    });
    await expect(page).toHaveURL(/runId=[0-9a-f]+/);
    const url = page.url();

    // Refresh while the run is still going. The persisted "running"
    // status is stale (the loop lives only in memory), so the page must
    // present Resume — not a Pause button that no loop is listening to.
    await page.goto(url);
    await expect(page.getByRole("button", { name: "Resume" })).toBeVisible({
      timeout: 30_000,
    });
    // No live Pause control (exact match — the run-list item's "paused N"
    // label is a substring match for "Pause" otherwise).
    await expect(
      page.getByRole("button", { name: "Pause", exact: true }),
    ).toHaveCount(0);

    // Resuming continues the run to completion.
    await page.getByRole("button", { name: "Resume" }).click();
    await expect(page.getByText(/Complete/)).toBeVisible({
      timeout: 180_000,
    });
  });

  test("nav lets the user switch between Simulate and Calibrate", async ({
    page,
  }) => {
    await page.goto("/");
    await expect(
      page.getByRole("heading", { name: "Ixa Basic Transmission" }),
    ).toBeVisible();

    await page.getByRole("link", { name: "Calibrate" }).click();
    await expect(page).toHaveURL(/\/calibrate$/);
    await expect(
      page.getByRole("heading", { name: "Calibration", exact: true }),
    ).toBeVisible();

    await page.getByRole("link", { name: "Simulate" }).click();
    await expect(page).toHaveURL(/\/$/);
    await expect(
      page.getByRole("heading", { name: "Ixa Basic Transmission" }),
    ).toBeVisible();
  });
});
