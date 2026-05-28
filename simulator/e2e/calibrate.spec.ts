import { test, expect, type Page } from "@playwright/test";

// /calibrate end-to-end smoke. Confirms:
//   1. The page loads with the calibration sidebar and the calibrate nav link.
//   2. "Generate from parameters" populates the target-data table from a
//      model run; "Start new run" then runs ABC-SMC to completion and
//      renders per-stage cells with a trajectory overlay + R₀ +
//      Initial-infections histograms.
//   3. The URL gains `?runId=…` after starting and a fresh navigation to
//      that URL deep-links back into the same run (loaded from IndexedDB).
//
// The wasm worker is built lazily on first navigation; we extend the
// per-action timeout to absorb both the cold-start build and the multi-
// stage ABC-SMC run.

// Target data is empty by default (manual entry). Populate it from the
// model parameters so a run has something to calibrate against. Resolves
// once the editor reflects the generated rows.
async function generateTargetData(page: Page) {
  await page.getByRole("button", { name: "Generate from parameters" }).click();
  // The model run builds the wasm worker on first use; wait for the
  // populated table (row 1 appears) before continuing.
  await expect(page.getByLabel("Incident cases for row 1", { exact: true })).toBeVisible({
    timeout: 120_000,
  });
}

// Creating a run and starting it are now two separate actions: "Create new
// run" mints a fresh idle run, then "Start" runs it.
async function createAndStart(page: Page) {
  await page.getByRole("button", { name: "Create new run" }).click();
  await page.getByRole("button", { name: "Start", exact: true }).click();
}

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

    // Target data is empty by default; populate it from the model params.
    await generateTargetData(page);

    // Create the run, then start it. Default config is 100 particles ×
    // (1 prior + 4 perturb stages); on a CI machine this can take ~30s.
    await createAndStart(page);

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

    // Populate target data, create + start a run, wait until it's in flight.
    await generateTargetData(page);
    await createAndStart(page);
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

  test("params are locked once a run starts and stays locked when complete", async ({
    page,
  }) => {
    await page.goto("/calibrate");
    await expect(
      page.getByRole("heading", { name: "Calibration", exact: true }),
    ).toBeVisible();

    // A fresh (idle) run is editable.
    const population = page.getByLabel("Population", { exact: true });
    await expect(population).toBeEnabled();
    await expect(page.getByText("This run is read-only")).toHaveCount(0);

    // The target-data editor is available while idle — including after
    // "Create new run" (a freshly-created run is still idle/editable).
    await generateTargetData(page);
    await page.getByRole("button", { name: "Create new run" }).click();
    await expect(population).toBeEnabled();
    await expect(
      page.getByRole("button", { name: "Generate from parameters" }),
    ).toBeVisible();

    // Starting the run freezes the params and hides the data editor.
    await page.getByRole("button", { name: "Start", exact: true }).click();
    await expect(page.getByText("This run is read-only")).toBeVisible({
      timeout: 60_000,
    });
    await expect(population).toBeDisabled();
    await expect(
      page.getByRole("button", { name: "Generate from parameters" }),
    ).toHaveCount(0);
    await expect(page.getByLabel("Incident cases for row 1", { exact: true })).toHaveCount(0);

    // Still locked after it completes.
    await expect(page.getByText(/Complete/)).toBeVisible({ timeout: 180_000 });
    await expect(population).toBeDisabled();
    await expect(page.getByText("This run is read-only")).toBeVisible();

    // "+ New" mints a fresh idle run → editable again.
    await page.getByRole("button", { name: "+ New" }).click();
    await expect(population).toBeEnabled();
    await expect(page.getByText("This run is read-only")).toHaveCount(0);
  });

  test("target editor: manual rows, editable days with gaps, auto-sort", async ({
    page,
  }) => {
    await page.goto("/calibrate");
    await expect(
      page.getByRole("heading", { name: "Calibration", exact: true }),
    ).toBeVisible();

    // The data table is shown by default (no source dropdown / toggle).
    await expect(page.getByRole("columnheader", { name: "Day" })).toBeVisible();

    // Build three rows by hand. New rows default to the next contiguous
    // day (1, 2, 3).
    const addRow = page.getByRole("button", { name: "Add row" });
    await addRow.click();
    await addRow.click();
    await addRow.click();

    await page.getByLabel("Incident cases for row 1", { exact: true }).fill("5");
    await page.getByLabel("Incident cases for row 2", { exact: true }).fill("8");
    await page.getByLabel("Incident cases for row 3", { exact: true }).fill("3");

    // Days are editable (gaps allowed). Push row 1 to day 9 — out of
    // order — then blur to commit, which auto-sorts the table by day so
    // the rows become (2, 8), (3, 3), (9, 5).
    await page.getByLabel("Day for row 1", { exact: true }).fill("9");
    await page.getByLabel("Incident cases for row 3", { exact: true }).click();

    await expect(page.getByLabel("Day for row 1", { exact: true })).toHaveValue("2");
    await expect(page.getByLabel("Incident cases for row 1", { exact: true })).toHaveValue("8");
    await expect(page.getByLabel("Day for row 2", { exact: true })).toHaveValue("3");
    await expect(page.getByLabel("Incident cases for row 2", { exact: true })).toHaveValue("3");
    await expect(page.getByLabel("Day for row 3", { exact: true })).toHaveValue("9");
    await expect(page.getByLabel("Incident cases for row 3", { exact: true })).toHaveValue("5");

    // Removing the middle row drops it and leaves days 2 and 9.
    await page.getByRole("button", { name: "Remove row 2" }).click();
    await expect(page.getByLabel("Day for row 1", { exact: true })).toHaveValue("2");
    await expect(page.getByLabel("Day for row 2", { exact: true })).toHaveValue("9");
    await expect(page.getByLabel("Day for row 3", { exact: true })).toHaveCount(0);

    // Clear empties the table back to a blank slate.
    await page.getByRole("button", { name: "Clear" }).click();
    await expect(page.getByLabel("Day for row 1", { exact: true })).toHaveCount(0);
    await expect(page.getByRole("button", { name: "Clear" })).toBeDisabled();
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
