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
    // is value × duration = 0.3 × 5 = 1.50.
    await expect(page.getByText("R₀ (expected)")).toBeVisible();
    await expect(page.getByText("1.50")).toBeVisible();
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
});
