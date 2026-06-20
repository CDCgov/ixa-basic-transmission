import { test, expect } from "@playwright/test";

// Verifies the preset picker in the sidebar:
//   - lists every preset from `config/*.{toml,json}`
//   - starts with "Baseline" selected (matches defaults)
//   - applying a preset updates params (URL query string + form fields)
//   - editing a field afterwards drops the picker back to "Custom"
test.describe("Parameter presets", () => {
  test("preset picker lists, applies, and clears on edit", async ({
    page,
  }) => {
    await page.goto("/");

    const preset = page.getByRole("combobox", { name: "Preset" });
    await expect(preset).toBeVisible();
    await expect(preset).toHaveText(/Baseline/);
    await expect(
      page.getByText(/Default scenario:/),
    ).toBeVisible();

    await preset.click();
    const options = page.getByRole("option");
    await expect(options).toHaveText([
      "Baseline",
      "Fast outbreak",
      "Library of empirical curves",
      "Slow spread",
      "Time-varying infectiousness",
      "Tiny town",
    ]);

    await page.getByRole("option", { name: "Fast outbreak" }).click();

    // URL only carries values that differ from defaults; Fast outbreak
    // bumps maxTime. `infectionRate` is excluded from URL sync (it's a
    // tagged union and the sync layer can only enumerate one variant of
    // defaults at a time — see issue filed upstream).
    await expect(page).toHaveURL(/maxTime=60/);
    await expect(preset).toHaveText(/Fast outbreak/);

    // Tiny town (JSON preset) — exercises the JSON parsing path.
    await preset.click();
    await page.getByRole("option", { name: "Tiny town" }).click();
    await expect(page).toHaveURL(/population=500/);
    await expect(preset).toHaveText(/Tiny town/);

    // Editing any param away from the preset clears the selection.
    const seed = page.getByRole("textbox", { name: "Seed" });
    await seed.fill("7");
    await seed.blur();
    await expect(preset).toHaveText(/Custom/);
  });

  test("Infectiousness selector switches between Constant and Empirical", async ({
    page,
  }) => {
    await page.goto("/");
    const rateType = page.getByRole("combobox", { name: "Infectiousness" });
    await expect(rateType).toHaveText(/Constant/);
    // Constant-mode labels for the rate value and the recovery period.
    await expect(page.getByText("Infection rate")).toBeVisible();
    await expect(page.getByText("Infectious period")).toBeVisible();

    // Switch to empirical: those slider labels are gone, the curve
    // summary appears (default 5-anchor viral-load curve), and the
    // points editor is rendered.
    await rateType.click();
    await page.getByRole("option", { name: "Empirical", exact: true }).click();
    await expect(page.getByText("Infection rate")).toHaveCount(0);
    await expect(
      page.getByText(/Empirical curve, recovery at τ =/i),
    ).toBeVisible();
    await expect(page.getByRole("button", { name: "Add point" })).toBeVisible();

    // Switch back to constant: sliders return.
    await rateType.click();
    await page.getByRole("option", { name: "Constant" }).click();
    await expect(page.getByText("Infection rate")).toBeVisible();
  });

  test("empirical schedule preset shows curve summary", async ({ page }) => {
    await page.goto("/");
    const preset = page.getByRole("combobox", { name: "Preset" });
    await preset.click();
    await page
      .getByRole("option", { name: "Time-varying infectiousness" })
      .click();

    const rateType = page.getByRole("combobox", { name: "Infectiousness" });
    await expect(rateType).toHaveText(/Empirical/);
    await expect(
      page.getByText(/Empirical curve, recovery at τ =/i),
    ).toBeVisible();
  });

  test("Library preset switches the editor into Library mode and shows the mean chart", async ({
    page,
  }) => {
    await page.goto("/");
    const preset = page.getByRole("combobox", { name: "Preset" });
    await preset.click();
    await page
      .getByRole("option", { name: "Library of empirical curves" })
      .click();
    await expect(preset).toHaveText(/Library of empirical curves/);
    const rateType = page.getByRole("combobox", { name: "Infectiousness" });
    await expect(rateType).toHaveText(/Library/);
    await expect(
      page.getByText(/Library of 10 per-person curves/i),
    ).toBeVisible();
    // The default view is "Mean" — a single chart with all curves and
    // a red mean overlay. Pagination is hidden in this view.
    const view = page.getByRole("combobox", { name: "View" });
    await expect(view).toHaveText(/Mean/);
    await expect(page.getByText(/Page \d+ \/ \d+/)).toHaveCount(0);
  });

  test("simulation runs end-to-end in Library mode and shows summary stats", async ({
    page,
  }) => {
    await page.goto("/");
    const rateType = page.getByRole("combobox", { name: "Infectiousness" });
    await rateType.click();
    await page.getByRole("option", { name: "Library" }).click();

    // Library mode kicks off a fresh simulation. Wait for completion.
    await expect(page.getByText(/Ran \d+ simulations/)).toBeVisible({
      timeout: 60_000,
    });
    await expect(page.getByText("Cumulative infections")).toBeVisible();
    await expect(
      page.getByText("Incidence", { exact: true }),
    ).toBeVisible();
    // Summary table still shows the two standard metrics — R₀ is the
    // mean curve area across the bundled library.
    await expect(page.getByText("R₀ (expected)")).toBeVisible();
    await expect(
      page.getByText("Attack rate (observed median)"),
    ).toBeVisible();
  });

  test("Library mode rejects malformed CSV with an inline error", async ({
    page,
  }) => {
    await page.goto("/");
    const rateType = page.getByRole("combobox", { name: "Infectiousness" });
    await rateType.click();
    await page.getByRole("option", { name: "Library" }).click();

    // Upload garbage — no numeric columns, no header. Parser must reject.
    const garbage = "this is not a csv\njust some words\n";
    await page
      .locator("input[type='file']")
      .setInputFiles({
        name: "broken.csv",
        mimeType: "text/csv",
        buffer: Buffer.from(garbage),
      });
    // The library should be unchanged (10 curves still showing) and
    // an error message should be visible.
    await expect(
      page.getByText(/Library of 10 per-person curves/i),
    ).toBeVisible();
    await expect(
      page.getByText(/non-numeric|expected 3 columns|no data rows/i),
    ).toBeVisible();
  });

  test("library mode paginates through curves and accepts a CSV upload", async ({
    page,
  }) => {
    await page.goto("/");
    const rateType = page.getByRole("combobox", { name: "Infectiousness" });
    await rateType.click();
    await page.getByRole("option", { name: "Library" }).click();

    // Default view is "Overlay" (single chart with all curves + red
    // mean). Switch to "Grid" to test pagination.
    await expect(
      page.getByText(/Library of 10 per-person curves/i),
    ).toBeVisible();
    const view = page.getByRole("combobox", { name: "View" });
    await view.click();
    await page.getByRole("option", { name: "Grid" }).click();

    // The bundled default library has 10 curves; with 4 per page that's
    // 3 pages of 4-4-2.
    await expect(page.getByText(/Page 1 \/ 3/)).toBeVisible();

    // Forward through pages.
    await page.getByRole("button", { name: /Next/ }).click();
    await expect(page.getByText(/Page 2 \/ 3/)).toBeVisible();
    await page.getByRole("button", { name: /Next/ }).click();
    await expect(page.getByText(/Page 3 \/ 3/)).toBeVisible();
    // Next is disabled at the end.
    await expect(page.getByRole("button", { name: /Next/ })).toBeDisabled();
    await page.getByRole("button", { name: /Prev/ }).click();
    await expect(page.getByText(/Page 2 \/ 3/)).toBeVisible();

    // Upload a small CSV → library is swapped (2 curves → 1 page).
    const csv = "id,time,value\n1,0,0.1\n1,1,0.5\n2,0,0.2\n2,2,0.6\n";
    await page
      .locator("input[type='file']")
      .setInputFiles({
        name: "rates.csv",
        mimeType: "text/csv",
        buffer: Buffer.from(csv),
      });
    await expect(
      page.getByText(/Library of 2 per-person curves/i),
    ).toBeVisible();
    // No pager when curves <= page size.
    await expect(page.getByText(/Page \d+ \/ \d+/)).toHaveCount(0);

    // Restore default brings 10 curves back.
    await page.getByRole("button", { name: "Restore default" }).click();
    await expect(
      page.getByText(/Library of 10 per-person curves/i),
    ).toBeVisible();
  });

  test("points editor adds and removes anchor points", async ({ page }) => {
    await page.goto("/");
    const rateType = page.getByRole("combobox", { name: "Infectiousness" });
    await rateType.click();
    await page.getByRole("option", { name: "Empirical", exact: true }).click();

    // Default seeded curve has 5 anchor rows in the editor.
    await expect(page.getByRole("button", { name: "×" })).toHaveCount(5);

    // Add a point → 6 rows.
    await page.getByRole("button", { name: "Add point" }).click();
    await expect(page.getByRole("button", { name: "×" })).toHaveCount(6);

    // Remove (the first × button) → back to 5.
    await page.getByRole("button", { name: "×" }).first().click();
    await expect(page.getByRole("button", { name: "×" })).toHaveCount(5);
  });
});
