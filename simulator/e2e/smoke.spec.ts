import { test, expect } from "@playwright/test";

// Top-level smoke + cross-cutting UI tests:
//   - the page loads, kicks off a simulation, and renders charts + summary
//   - the JSON code editor round-trips a parameter change back to the form
//   - the Reset button restores defaults
//   - deep-linking via URL params reproduces a preset's state on first load
//
// The wasm worker is built lazily on first navigation, so the first
// `goto` uses Playwright's default action timeout (extended in
// `playwright.config.ts`'s webServer timeout).

test.describe("Simulator smoke", () => {
  test("loads, runs a simulation, and renders charts + summary", async ({
    page,
  }) => {
    await page.goto("/");

    await expect(
      page.getByRole("heading", { name: "Ixa Basic Transmission" }),
    ).toBeVisible();

    // Status line transitions through "Running…" → "Ran N simulations".
    // Allow generous time because the wasm module is compiled on first
    // request in dev.
    await expect(page.getByText(/Ran \d+ simulations/)).toBeVisible({
      timeout: 60_000,
    });

    // Both charts mount with y-axis labels we set in `Page.vue`.
    await expect(page.getByText("Cumulative infections")).toBeVisible();
    await expect(
      page.getByText("Incidence (new infections per time unit)"),
    ).toBeVisible();

    // Summary table shows the two metrics. The R₀ for the Baseline preset
    // is value × duration = 0.5 × 3 = 1.50.
    await expect(page.getByText("R₀ (expected)")).toBeVisible();
    await expect(page.getByText("1.50").first()).toBeVisible();
    await expect(
      page.getByText("Attack rate (observed median)"),
    ).toBeVisible();
    // Attack rate is stochastic but rendered as a percentage.
    await expect(page.getByText(/\d+\.\d%/)).toBeVisible();
  });

  test("Reset button restores defaults from a custom state", async ({
    page,
  }) => {
    await page.goto("/");
    const preset = page.getByRole("combobox", { name: "Preset" });
    await expect(preset).toHaveText(/Baseline/);

    // Edit a param away from defaults → preset goes to Custom, URL gains
    // the changed value.
    const seed = page.getByRole("textbox", { name: "Seed" });
    await seed.fill("42");
    await seed.blur();
    await expect(preset).toHaveText(/Custom/);
    await expect(page).toHaveURL(/seed=42/);

    // Reset should clear the URL params and put us back on Baseline.
    await page.getByRole("button", { name: "Reset" }).click();
    await expect(preset).toHaveText(/Baseline/);
    await expect(seed).toHaveValue("0");
    await expect(page).not.toHaveURL(/seed=/);
  });

  test("URL params deep-link to a preset's state on first load", async ({
    page,
  }) => {
    // Tiny town preset: population 500, initialInfections 2 (per
    // config/tiny-town.json). Loading those via URL on first visit should
    // resolve the picker to that preset.
    await page.goto("/?population=500&initialInfections=2");
    const preset = page.getByRole("combobox", { name: "Preset" });
    await expect(preset).toHaveText(/Tiny town/);
    await expect(
      page.getByRole("textbox", { name: "Population" }),
    ).toHaveValue("500");
  });

  test("infectionRate round-trips through the URL across variants", async ({
    page,
  }) => {
    await page.goto("/");
    // Switching from Constant → Time-varying should write
    // `infectionRate=<json>` into the URL (the tagged-union codec).
    const rateType = page.getByRole("combobox", { name: "Infectiousness" });
    await rateType.click();
    await page.getByRole("option", { name: "Time-varying" }).click();
    await expect(page).toHaveURL(/infectionRate=.*empirical/);

    // Library mode serializes a compact `{"type":"library","scale":…}`
    // tag — the bundled curves are restored on deserialize, so the URL
    // stays short while `scale` (the calibration) round-trips.
    await rateType.click();
    await page.getByRole("option", { name: "Library" }).click();
    await expect(page).toHaveURL(/infectionRate=.*library/);
    // The default library scale (3.0 — the bundled curves are mean-area
    // normalized so `scale` reads as expected R₀) must appear in the
    // encoded URL so it doesn't fall back to 1.0 on reload.
    await expect(page).toHaveURL(/scale.*3/);
    const libraryUrl = page.url();
    expect(libraryUrl.length).toBeLessThan(500);

    // Reload the page with that URL → editor comes back in Library mode
    // and the 10-curve grid is restored from the bundled default.
    await page.goto(libraryUrl);
    await expect(rateType).toHaveText(/Library/);
    await expect(
      page.getByText(/Library of 10 per-person curves/i),
    ).toBeVisible();
  });

  test("Edit-as-code toggle reveals the JSON editor and round-trips a change", async ({
    page,
  }) => {
    await page.goto("/");

    // Form view is the default — the per-field Seed input is visible.
    await expect(page.getByRole("textbox", { name: "Seed" })).toBeVisible();

    await page.getByLabel("Edit as code").click();

    // Editor swaps in. ParamEditor renders a Format select + Import /
    // Export / Apply buttons.
    await expect(
      page.getByRole("combobox", { name: "Format" }),
    ).toBeVisible();
    await expect(page.getByRole("button", { name: "Apply" })).toBeVisible();

    // The CodeMirror editor is a single role="textbox" inside the
    // editor container. Its text contains the current params as JSON.
    const editor = page.locator(".param-editor-cm [role='textbox']");
    await expect(editor).toContainText('"population": 10000');

    // Flip back to the form view and confirm the form is back.
    await page.getByLabel("Edit as code").click();
    await expect(page.getByRole("textbox", { name: "Seed" })).toBeVisible();
  });

  test("Library preset renders R₀ = scale (mean-area normalized curves)", async ({
    page,
  }) => {
    // The model's `normalize::normalize_to_r0` rescales the bundled
    // library so the mean curve has area 1 — `scale` then reads as
    // the expected R₀ under random mixing. Sanity check: picking the
    // Library preset (scale = 3.0) should render R₀ (expected) ≈ 3.00
    // in the summary table.
    await page.goto("/");
    const preset = page.getByRole("combobox", { name: "Preset" });
    await preset.click();
    await page
      .getByRole("option", { name: "Library of empirical curves" })
      .click();

    // Wait for the simulation to complete before asserting on the
    // summary table (the R₀ row only mounts after the first run).
    await expect(page.getByText(/Ran \d+ simulations/)).toBeVisible({
      timeout: 60_000,
    });

    await expect(page.getByText("R₀ (expected)")).toBeVisible();
    await expect(page.getByText("3.00", { exact: true })).toBeVisible();
  });

  test("Constant rate editor shows live R₀ readout above the preview", async ({
    page,
  }) => {
    // Constant has no R₀ slider (it's derived: value × duration), so
    // the editor renders an inline readout next to the preview chart.
    // Baseline preset: value = 0.5, duration = 3 → R₀ = 1.50.
    await page.goto("/");
    const readout = page
      .getByText("R₀ (random mixing)")
      .locator("xpath=..");
    await expect(readout).toContainText("1.50");

    // The readout reacts to slider input: bumping value via arrow keys
    // (step=0.05) one notch up → 0.55, R₀ = 0.55 × 3 = 1.65.
    const rate = page.getByRole("slider", { name: "Infection rate" });
    await rate.focus();
    await rate.press("ArrowRight");
    await expect(readout).toContainText("1.65");
  });

  test("Time-varying preset renders R₀ = scale (area-normalized curve)", async ({
    page,
  }) => {
    // Same contract as Library, single-curve variant: the empirical
    // points are area-normalized so `scale` is the expected R₀ under
    // random mixing. Time-varying preset's scale = 3.0 → R₀ ≈ 3.00.
    await page.goto("/");
    const preset = page.getByRole("combobox", { name: "Preset" });
    await preset.click();
    await page
      .getByRole("option", { name: "Time-varying infectiousness" })
      .click();

    await expect(page.getByText(/Ran \d+ simulations/)).toBeVisible({
      timeout: 60_000,
    });

    await expect(page.getByText("R₀ (expected)")).toBeVisible();
    await expect(page.getByText("3.00", { exact: true })).toBeVisible();
  });
});
