import { test, expect } from "@playwright/test";

// Sidebar rate preview: a small LineChart traces λ(τ) for both variants.
// The selector targets the `.rate-preview` wrapper inside `RateEditor.vue`.

test.describe("Rate preview", () => {
  test("renders an SVG line for Constant", async ({ page }) => {
    await page.goto("/");
    const preview = page.locator(".rate-preview");
    await expect(preview).toBeVisible();
    // LineChart renders an <svg> root.
    await expect(preview.locator("svg")).toBeVisible();
  });

  test("re-renders with dots when switching to Empirical", async ({
    page,
  }) => {
    await page.goto("/");
    await page.getByRole("combobox", { name: "Infectiousness" }).click();
    await page.getByRole("option", { name: "Empirical", exact: true }).click();
    const preview = page.locator(".rate-preview");
    // 5 default anchors → 5 dot circles. LineChart draws each dot as a
    // <circle> with the series color (#2563eb in our config).
    await expect(preview.locator('svg circle[fill="#2563eb"]')).toHaveCount(5);
  });
});
