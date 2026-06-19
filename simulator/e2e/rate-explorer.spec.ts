import { test, expect } from "@playwright/test";

// The main area splits into two tabs: "Simulate" (run charts) and
// "Explore rate" (an interactive explainer of the currently-selected rate
// function — λ(τ) area + inverse-CDF sampling of event times). The sidebar
// stays mounted across both, so the explorer reflects live rate edits.

test.describe("Rate explorer tab", () => {
  test("Explore tab shows the explainer and samples event times", async ({
    page,
  }) => {
    await page.goto("/");
    await page.getByRole("tab", { name: "Explore rate" }).click();

    // Header reflects the default Constant rate.
    await expect(
      page.getByRole("heading", { name: "Constant rate" }),
    ).toBeVisible();

    // Empty until the first draw — no table yet.
    const draw = page.getByRole("button", { name: "Draw next" });
    await expect(draw).toBeVisible();
    await expect(page.locator(".explorer-table")).toHaveCount(0);

    // No draws yet → the running title and timeline are hidden.
    await expect(page.locator(".timeline-title")).toBeHidden();
    await expect(page.locator(".timeline-svg")).toBeHidden();

    // Draw → a row appears (an event time, or "recovered" if the draw
    // overshoots the curve area — either way the table renders), and the
    // title + SVG timeline appear.
    await draw.click();
    await expect(page.locator(".explorer-table tbody tr")).not.toHaveCount(0);
    await expect(page.locator(".timeline-title")).toContainText("total infection");
    await expect(page.locator(".timeline-svg")).toBeVisible();

    // Reset (scoped to the explorer — the sidebar also has a Reset) clears the
    // table and hides the timeline again.
    await page
      .locator(".explorer")
      .getByRole("button", { name: "Reset" })
      .click();
    await expect(page.locator(".explorer-table")).toHaveCount(0);
    await expect(page.locator(".timeline-title")).toBeHidden();
  });

  test("recovery marker appears once the draws exhaust the curve", async ({
    page,
  }) => {
    await page.goto("/");
    await page.getByRole("tab", { name: "Explore rate" }).click();
    const draw = page.getByRole("button", { name: "Draw next" });
    // Default Constant rate has total area 1.5, so a few Exp(1) draws
    // overshoot it; keep drawing until the button disables (recovered).
    for (let i = 0; i < 40 && (await draw.isEnabled()); i++) {
      await draw.click();
    }
    await expect(draw).toBeDisabled();
    await expect(page.locator(".timeline-recovery")).toBeVisible();
    await expect(page.locator(".timeline-title-recovered")).toBeVisible();
  });

  test("constant rate offers a homogeneous-Poisson toggle that swaps the view", async ({
    page,
  }) => {
    await page.goto("/");
    await page.getByRole("tab", { name: "Explore rate" }).click();

    // Default constant rate → simplified homogeneous view: the toggle is on
    // and the table uses the Exp(rate) "gap" column.
    const toggle = page.getByRole("switch", {
      name: "Homogeneous Poisson view",
    });
    await expect(toggle).toBeVisible();
    await page.getByRole("button", { name: "Draw next" }).click();
    await expect(
      page.getByRole("columnheader", { name: /gap/ }),
    ).toBeVisible();

    // Toggle off → general inverse-CDF view: c(t) chart + the Exp(1) column.
    await toggle.click();
    await expect(
      page.getByRole("heading", { name: "Cumulative rate c(t)" }),
    ).toBeVisible();
    await expect(
      page.getByRole("columnheader", { name: /Exp\(1\)/ }),
    ).toBeVisible();
  });

  test("a time-varying rate has no homogeneous toggle", async ({ page }) => {
    await page.goto("/");
    await page.getByRole("combobox", { name: "Infectiousness" }).click();
    await page.getByRole("option", { name: "Parametric", exact: true }).click();
    await page.getByRole("tab", { name: "Explore rate" }).click();
    await expect(
      page.getByRole("switch", { name: "Homogeneous Poisson view" }),
    ).toHaveCount(0);
  });

  test("explorer reflects the selected rate type", async ({ page }) => {
    await page.goto("/");
    // Switch the rate to Parametric (Gamma default) in the sidebar.
    await page.getByRole("combobox", { name: "Infectiousness" }).click();
    await page.getByRole("option", { name: "Parametric", exact: true }).click();

    await page.getByRole("tab", { name: "Explore rate" }).click();
    await expect(page.getByRole("heading", { name: /Gamma/ })).toBeVisible();
  });

  test("switching back to Simulate shows the charts", async ({ page }) => {
    await page.goto("/");
    await page.getByRole("tab", { name: "Explore rate" }).click();
    await page.getByRole("tab", { name: "Simulate" }).click();
    await expect(page.getByText("Cumulative infections")).toBeVisible();
  });

  test("the Explore tab has its own URL path and deep-links on reload", async ({
    page,
  }) => {
    await page.goto("/");
    await page.getByRole("tab", { name: "Explore rate" }).click();
    await expect(page).toHaveURL(/\/explore/);

    // Reloading the /explore URL lands directly on the explorer.
    await page.reload();
    await expect(
      page.getByRole("heading", { name: "Constant rate" }),
    ).toBeVisible();

    // Back to Simulate drops the /explore segment.
    await page.getByRole("tab", { name: "Simulate" }).click();
    await expect(page).not.toHaveURL(/\/explore/);
    await expect(page.getByText("Cumulative infections")).toBeVisible();
  });

  test("deep-linking to /explore preserves model params in the query", async ({
    page,
  }) => {
    // A non-default param lives in the query string; switching to the
    // explorer path must carry it along.
    await page.goto("/?seed=42");
    await page.getByRole("tab", { name: "Explore rate" }).click();
    await expect(page).toHaveURL(/\/explore/);
    await expect(page).toHaveURL(/seed=42/);
  });
});
