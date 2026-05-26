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

  test("Infectiousness selector switches between Constant and Time-varying", async ({
    page,
  }) => {
    await page.goto("/");
    const rateType = page.getByRole("combobox", { name: "Infectiousness" });
    await expect(rateType).toHaveText(/Constant/);
    // Constant-mode labels for the rate value and the recovery period.
    await expect(page.getByText("Infection rate")).toBeVisible();
    await expect(page.getByText("Infectious period")).toBeVisible();

    // Switch to time-varying: those slider labels are gone, the curve
    // summary appears (default 5-anchor viral-load curve), and the
    // points editor is rendered.
    await rateType.click();
    await page.getByRole("option", { name: "Time-varying" }).click();
    await expect(page.getByText("Infection rate")).toHaveCount(0);
    await expect(
      page.getByText(/Time-varying curve, recovery at τ =/i),
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
    await expect(rateType).toHaveText(/Time-varying/);
    await expect(
      page.getByText(/Time-varying curve, recovery at τ =/i),
    ).toBeVisible();
  });

  test("points editor adds and removes anchor points", async ({ page }) => {
    await page.goto("/");
    const rateType = page.getByRole("combobox", { name: "Infectiousness" });
    await rateType.click();
    await page.getByRole("option", { name: "Time-varying" }).click();

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
