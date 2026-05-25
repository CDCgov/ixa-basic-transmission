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
      "Tiny town",
    ]);

    await page.getByRole("option", { name: "Fast outbreak" }).click();

    // URL only carries values that differ from defaults; Fast outbreak
    // bumps infectionRate, infectiousPeriod, and maxTime.
    await expect(page).toHaveURL(/infectionRate=0\.6/);
    await expect(page).toHaveURL(/infectiousPeriod=5/);
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
});
