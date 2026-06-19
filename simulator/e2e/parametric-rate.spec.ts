import { test, expect } from "@playwright/test";

// Parametric (distribution-based) infectiousness rate:
//   - selecting "Parametric" reveals the distribution family selector +
//     param fields and the preview renders a smooth line (no anchor dots)
//   - the config round-trips through the URL on reload
//   - a parametric config completes a simulation end-to-end (proves the
//     wasm lowers Parametric → Empirical and runs)

test.describe("Parametric infectiousness rate", () => {
  test("selecting Parametric reveals fields and renders a preview line", async ({
    page,
  }) => {
    await page.goto("/");
    await page.getByRole("combobox", { name: "Infectiousness" }).click();
    await page.getByRole("option", { name: "Parametric", exact: true }).click();

    // Family selector defaults to Gamma; switching is possible.
    const family = page.getByRole("combobox", { name: "Distribution" });
    await expect(family).toHaveText(/Gamma/);

    // Gamma exposes Shape + Scale param inputs plus the R₀ scale slider.
    await expect(page.getByLabel("Shape")).toBeVisible();
    await expect(page.getByText("Scale (R₀)")).toBeVisible();

    // The preview is a smooth line — no anchor dots (unlike Empirical).
    const preview = page.locator(".rate-preview");
    await expect(preview.locator("svg")).toBeVisible();
    await expect(preview.locator('svg circle[fill="#2563eb"]')).toHaveCount(0);

    // Switching family to Lognormal swaps the param fields to μ / σ.
    await family.click();
    await page.getByRole("option", { name: "Lognormal" }).click();
    await expect(page.getByLabel("μ (log-mean)")).toBeVisible();
    await expect(page.getByLabel("σ (log-SD)")).toBeVisible();
  });

  test("round-trips through the URL on reload", async ({ page }) => {
    await page.goto("/");
    await page.getByRole("combobox", { name: "Infectiousness" }).click();
    await page.getByRole("option", { name: "Parametric", exact: true }).click();

    // The URL gains a parametric infectionRate param.
    await expect(page).toHaveURL(/infectionRate=.*parametric/);

    await page.reload();
    // After reload the editor is still on Parametric / Gamma.
    await expect(
      page.getByRole("combobox", { name: "Infectiousness" }),
    ).toHaveText(/Parametric/);
    await expect(
      page.getByRole("combobox", { name: "Distribution" }),
    ).toHaveText(/Gamma/);
  });

  test("completes a simulation end-to-end", async ({ page }) => {
    await page.goto("/");
    await page.getByRole("combobox", { name: "Infectiousness" }).click();
    await page.getByRole("option", { name: "Parametric", exact: true }).click();

    // The run kicks off and finishes (wasm lowers Parametric → Empirical).
    await expect(page.getByText(/Ran \d+ simulations/)).toBeVisible({
      timeout: 60_000,
    });
    await expect(page.getByText("Cumulative infections")).toBeVisible();
  });
});
