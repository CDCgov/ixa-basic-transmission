import { test, expect } from "@playwright/test";

// The "Modifiers" tab (/modifiers): rejection sampling vs rescheduling of a modifier
// that switches on mid-infection, driven by shared Draw next / Simulate 100×.

test.describe("Modifier explorer tab", () => {
  test("Modifiers tab shows both methods and draws advance each walk", async ({
    page,
  }) => {
    await page.goto("/");
    await page.getByRole("tab", { name: "Modifiers", exact: true }).click();
    await expect(page).toHaveURL(/\/modifiers/);

    await expect(
      page.getByRole("heading", { name: "Transmission modifiers" }),
    ).toBeVisible();
    await expect(
      page.getByRole("heading", { name: "Rejection sampling" }),
    ).toBeVisible();
    await expect(
      page.getByRole("heading", { name: "Rescheduling", exact: true }),
    ).toBeVisible();

    await expect(page.locator(".mexp-table")).toHaveCount(0);
    await expect(page.locator(".mexp-timeline-thin")).toBeHidden();
    await expect(page.locator(".mexp-summary")).toHaveCount(0);

    await page.locator(".mexp").getByRole("button", { name: "Draw next" }).click();
    await expect(
      page.locator(".mexp-panel-thin .mexp-table tbody tr"),
    ).toHaveCount(1);
    // A cancellation at tₘ and its redraw land in the same click.
    await expect(
      page.locator(".mexp-panel-resched .mexp-table tbody tr"),
    ).not.toHaveCount(0);
    await expect(page.locator(".mexp-timeline-thin")).toBeVisible();
    await expect(page.locator(".mexp-timeline-resched")).toBeVisible();

    // Scoped: the sidebar has its own Reset.
    await page.locator(".mexp").getByRole("button", { name: "Reset" }).click();
    await expect(page.locator(".mexp-table")).toHaveCount(0);
    await expect(page.locator(".mexp-timeline-thin")).toBeHidden();
  });

  test("Simulate 100× pools both methods into the equivalence summary", async ({
    page,
  }) => {
    await page.goto("/modifiers");
    await page.getByRole("button", { name: "Simulate 100×" }).click();

    const summary = page.locator(".mexp-summary");
    await expect(summary).toBeVisible();
    const rows = summary.locator("tbody tr");
    await expect(rows).toHaveCount(2);
    await expect(rows.nth(0)).toContainText("Rejection sampling");
    await expect(rows.nth(0)).toContainText("100");
    await expect(rows.nth(0)).toContainText("rejected attempts");
    await expect(rows.nth(1)).toContainText("Rescheduling");
    await expect(rows.nth(1)).toContainText("cancelled forecasts");
    // Default rate 0.5 × 3, on at day 1, f = 0.5: area 0.5 + 0.5 = 1.
    await expect(
      page.getByRole("heading", { name: "Distribution of simulated event times" }),
    ).toBeVisible();
    await expect(page.locator(".mexp-equiv")).toContainText(/shaded area 1(?!\.\d)/);
    await expect(page.locator(".mexp-equiv svg").first()).toBeVisible();

    await expect(
      page.locator(".mexp-timeline-thin .mtl-sim-dot").first(),
    ).toBeVisible();
    await expect(
      page.locator(".mexp-timeline-resched .mtl-sim-dot").first(),
    ).toBeVisible();

    // Drawing after a round keeps the pooled distribution.
    await page.locator(".mexp").getByRole("button", { name: "Draw next" }).click();
    await expect(page.locator(".mtl-sim-dot")).toHaveCount(0);
    await expect(summary).toBeVisible();
  });

  test("f > 1 greys out rejection sampling and runs rescheduling only", async ({
    page,
  }) => {
    await page.goto("/modifiers");
    const thin = page.locator(".mexp-panel-thin");
    await expect(thin).not.toHaveClass(/mexp-panel-off/);

    const f = page.getByRole("slider", { name: "Infectiousness multiplier f" });
    await f.focus();
    await page.keyboard.press("End");
    await expect(f).toHaveAttribute("aria-valuenow", "2");

    await expect(thin).toHaveClass(/mexp-panel-off/);
    await expect(thin.locator(".mexp-off-note")).toContainText(
      "Only rescheduling can be used",
    );

    await page.locator(".mexp").getByRole("button", { name: "Draw next" }).click();
    await expect(thin.locator(".mexp-table")).toHaveCount(0);
    await expect(
      page.locator(".mexp-panel-resched .mexp-table tbody tr"),
    ).not.toHaveCount(0);
    await page.getByRole("button", { name: "Simulate 100×" }).click();
    const rows = page.locator(".mexp-summary tbody tr");
    await expect(rows).toHaveCount(1);
    await expect(rows.first()).toContainText("Rescheduling");
    // Default rate 0.5 × 3, on at day 1, f = 2: area 0.5 + 2 = 2.5.
    await expect(page.locator(".mexp-equiv")).toContainText("shaded area 2.5");
  });
});
