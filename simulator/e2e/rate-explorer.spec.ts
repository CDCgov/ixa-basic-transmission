import { test, expect } from "@playwright/test";

// The main area splits into two tabs: "Simulate" (run charts) and
// "Explore" (an interactive explainer of the currently-selected rate
// function — λ(τ) area + inverse-CDF sampling of event times). The sidebar
// stays mounted across both, so the explorer reflects live rate edits.

test.describe("Rate explorer tab", () => {
  test("Explore tab shows the explainer and samples event times", async ({
    page,
  }) => {
    await page.goto("/");
    await page.getByRole("tab", { name: "Explore", exact: true }).click();

    // Header titles the explorer; the subtitle reflects the default Constant rate.
    await expect(
      page.getByRole("heading", { name: "Intrinsic Infectiousness" }),
    ).toBeVisible();
    await expect(page.locator(".explorer-subtitle")).toContainText(
      "Constant rate",
    );

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
    await expect(page.locator(".timeline-title")).toContainText("infection");
    await expect(page.locator(".timeline-svg")).toBeVisible();

    // Reset (scoped to the explorer — the sidebar also has a Reset) clears the
    // table, timeline, and distribution.
    await page
      .locator(".explorer")
      .getByRole("button", { name: "Reset" })
      .click();
    await expect(page.locator(".explorer-table")).toHaveCount(0);
    await expect(page.locator(".timeline-title")).toBeHidden();
    await expect(page.locator(".explorer-sim")).toHaveCount(0);
  });

  test("Draw next keeps adding; an individual eventually recovers", async ({
    page,
  }) => {
    await page.goto("/");
    await page.getByRole("tab", { name: "Explore", exact: true }).click();
    const draw = page.getByRole("button", { name: "Draw next" });
    // Default Constant rate has total area 1.5, so individuals overshoot within
    // a couple of Exp(1) draws. Draw next never disables now — it rolls over to
    // a new individual — so keep drawing until a recovered round is showing.
    let recovered = false;
    for (let i = 0; i < 40 && !recovered; i++) {
      await draw.click();
      recovered = await page.locator(".timeline-recovery").isVisible();
    }
    expect(recovered).toBe(true);
    await expect(page.locator(".timeline-title-recovered")).toBeVisible();
    await expect(draw).toBeEnabled(); // never gets stuck
  });

  test("Simulate 100× animates a round and builds the event-time distribution", async ({
    page,
  }) => {
    await page.goto("/");
    await page.getByRole("tab", { name: "Explore", exact: true }).click();

    // No samples yet → the distribution section is absent.
    await expect(page.locator(".explorer-sim")).toHaveCount(0);

    // Run a round → the distribution-of-event-times histogram appears and the
    // timeline shows that round's event-time dots.
    const draw = page.getByRole("button", { name: "Draw next" });
    await page.getByRole("button", { name: "Simulate 100×" }).click();
    const sim = page.locator(".explorer-sim");
    await expect(
      sim.getByRole("heading", { name: "Distribution of event times" }),
    ).toBeVisible();
    await expect(sim).toContainText("event time");
    await expect(sim.locator("svg")).toBeVisible();
    await expect(page.locator(".timeline-title")).toContainText("This round");
    await expect(page.locator(".timeline-svg .sim-dot").first()).toBeVisible();

    // Draw next disables during the animation; once it re-enables the round
    // has finished.
    await expect(draw).toBeEnabled();

    // Reset clears the distribution and timeline.
    await page
      .locator(".explorer")
      .getByRole("button", { name: "Reset" })
      .click();
    await expect(page.locator(".explorer-sim")).toHaveCount(0);
    await expect(page.locator(".timeline-title")).toBeHidden();
  });

  test("the distribution accumulates across rounds; the timeline shows only the current one", async ({
    page,
  }) => {
    await page.goto("/");
    await page.getByRole("tab", { name: "Explore", exact: true }).click();
    const draw = page.getByRole("button", { name: "Draw next" });

    // A simulate round fills the timeline with jittered sim dots + a distribution.
    await page.getByRole("button", { name: "Simulate 100×" }).click();
    await expect(page.locator(".timeline-svg .sim-dot").first()).toBeVisible();
    await expect(draw).toBeEnabled(); // wait out the animation
    const simDots = await page.locator(".timeline-svg .sim-dot").count();
    expect(simDots).toBeGreaterThan(0);

    // Draw next → the timeline switches to the current (draw) round, so the sim
    // round's dots are gone, but the distribution persists (accumulated).
    await draw.click();
    await expect(page.locator(".timeline-svg .sim-dot")).toHaveCount(0);
    await expect(page.locator(".explorer-sim")).toBeVisible();
    await expect(page.locator(".timeline-title")).not.toContainText("This round");
  });

  test("constant rate offers a Generalized toggle that swaps the view", async ({
    page,
  }) => {
    await page.goto("/");
    await page.getByRole("tab", { name: "Explore", exact: true }).click();

    // Default constant rate → simplified homogeneous view: the "Generalized"
    // toggle is off and the table uses the Exp(rate) "Δτ" column.
    const toggle = page.getByRole("switch", { name: "Generalized" });
    await expect(toggle).toBeVisible();
    await expect(toggle).not.toBeChecked();
    await page.getByRole("button", { name: "Draw next" }).click();
    await expect(
      page.getByRole("columnheader", { name: /Δτ/ }),
    ).toBeVisible();

    // Toggle on → general inverse-CDF view: c(t) chart + the Exp(1) column.
    await toggle.click();
    await expect(
      page.getByRole("heading", { name: "Cumulative rate c(t)" }),
    ).toBeVisible();
    await expect(
      page.getByRole("columnheader", { name: /Exp\(1\)/ }),
    ).toBeVisible();
  });

  test("the toggle sits inline with the rate description, below the title", async ({
    page,
  }) => {
    await page.setViewportSize({ width: 390, height: 800 });
    await page.goto("/");
    await page.getByRole("tab", { name: "Explore", exact: true }).click();

    const toggle = page.getByRole("switch", { name: "Generalized" });
    const title = page.getByRole("heading", { name: "Intrinsic Infectiousness" });
    const subtitle = page.locator(".explorer-subtitle");
    await expect(toggle).toBeVisible();

    const toggleBox = await toggle.boundingBox();
    const titleBox = await title.boundingBox();
    const subtitleBox = await subtitle.boundingBox();
    expect(toggleBox).not.toBeNull();
    expect(titleBox).not.toBeNull();
    expect(subtitleBox).not.toBeNull();
    // The toggle is grouped with the description, below the title.
    expect(toggleBox!.y).toBeGreaterThanOrEqual(
      titleBox!.y + titleBox!.height - 2,
    );
    expect(toggleBox!.y).toBeGreaterThanOrEqual(subtitleBox!.y - 2);
  });

  test("a time-varying rate has no Generalized toggle", async ({ page }) => {
    await page.goto("/");
    await page.getByRole("combobox", { name: "Infectiousness" }).click();
    await page.getByRole("option", { name: "Parametric", exact: true }).click();
    await page.getByRole("tab", { name: "Explore", exact: true }).click();
    await expect(
      page.getByRole("switch", { name: "Generalized" }),
    ).toHaveCount(0);
  });

  test("explorer reflects the selected rate type", async ({ page }) => {
    await page.goto("/");
    // Switch the rate to Parametric (Gamma default) in the sidebar.
    await page.getByRole("combobox", { name: "Infectiousness" }).click();
    await page.getByRole("option", { name: "Parametric", exact: true }).click();

    await page.getByRole("tab", { name: "Explore", exact: true }).click();
    await expect(page.locator(".explorer-subtitle")).toContainText("Gamma");
  });

  test("a shared rate notes every individual gets the same function", async ({
    page,
  }) => {
    await page.goto("/");
    await page.getByRole("tab", { name: "Explore", exact: true }).click();
    await expect(page.locator(".explorer-note-text")).toHaveText(
      "Every individual gets the same rate function.",
    );
  });

  test("a library rate assigns a random curve, re-rollable via a button", async ({
    page,
  }) => {
    await page.goto("/");
    // Switch the rate to Library in the sidebar.
    await page.getByRole("combobox", { name: "Infectiousness" }).click();
    await page.getByRole("option", { name: "Library" }).click();

    await page.getByRole("tab", { name: "Explore", exact: true }).click();

    // The note explains each individual gets a random assigned curve.
    const note = page.locator(".explorer-note-text");
    await expect(note).toContainText("assigned a random curve");
    const before = await note.textContent();

    // The population overlay (all curves + red mean) renders below r(t).
    const overlay = page.locator(".explorer-overlay");
    await expect(overlay).toBeVisible();
    await expect(overlay.getByText("Library curves")).toBeVisible();
    await expect(overlay.locator("svg")).toBeVisible();

    // Re-roll → a different assigned curve (the bundled library has 10, and
    // the picker avoids repeating the current one).
    await page.getByRole("button", { name: "New random curve" }).click();
    await expect(note).not.toHaveText(before ?? "");
  });

  test("switching back to Simulate shows the charts", async ({ page }) => {
    await page.goto("/");
    await page.getByRole("tab", { name: "Explore", exact: true }).click();
    await page.getByRole("tab", { name: "Simulate" }).click();
    await expect(page.getByText("Cumulative infections")).toBeVisible();
  });

  test("the Explore tab has its own URL path and deep-links on reload", async ({
    page,
  }) => {
    await page.goto("/");
    await page.getByRole("tab", { name: "Explore", exact: true }).click();
    await expect(page).toHaveURL(/\/explore/);

    // Reloading the /explore URL lands directly on the explorer.
    await page.reload();
    await expect(
      page.getByRole("heading", { name: "Intrinsic Infectiousness" }),
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
    await page.getByRole("tab", { name: "Explore", exact: true }).click();
    await expect(page).toHaveURL(/\/explore/);
    await expect(page).toHaveURL(/seed=42/);
  });
});
