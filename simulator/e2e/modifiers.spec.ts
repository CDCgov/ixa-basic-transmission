import { test, expect } from "@playwright/test";

// Transmission-modifiers sidebar section: enabling a modifier reveals its
// fields, writes its config to the URL, and round-trips on reload;
// disabling clears it. Facemask and antiviral are independent and compose.

test.describe("Transmission modifiers", () => {
  test("facemask toggles on, reveals fields, round-trips through the URL", async ({
    page,
  }) => {
    await page.goto("/");

    await expect(
      page.getByRole("heading", { name: "Transmission modifiers" }),
    ).toBeVisible();

    const facemask = page.getByRole("switch", { name: "Facemask" });
    await expect(facemask).not.toBeChecked();
    // Fields are hidden (modifier disabled) — "Effectiveness" is unique to
    // the facemask card.
    await expect(
      page.getByRole("textbox", { name: "Effectiveness" }),
    ).toHaveCount(0);

    await facemask.click();
    await expect(facemask).toBeChecked();
    await expect(
      page.getByRole("textbox", { name: "Effectiveness" }),
    ).toBeVisible();
    await expect(page).toHaveURL(/facemask=/);

    // Reload from the encoded URL → facemask stays enabled.
    await page.goto(page.url());
    await expect(
      page.getByRole("switch", { name: "Facemask" }),
    ).toBeChecked();
    await expect(
      page.getByRole("textbox", { name: "Effectiveness" }),
    ).toBeVisible();

    // Disabling drops the param from the URL.
    await page.getByRole("switch", { name: "Facemask" }).click();
    await expect(page).not.toHaveURL(/facemask=/);
  });

  test("facemask and antiviral compose, both encoded in the URL", async ({
    page,
  }) => {
    await page.goto("/");

    await page.getByRole("switch", { name: "Facemask" }).click();
    await page.getByRole("switch", { name: "Antiviral treatment" }).click();

    // "Treatment delay" is unique to the antiviral card.
    await expect(
      page.getByRole("textbox", { name: "Treatment delay" }),
    ).toBeVisible();

    await expect(page).toHaveURL(/facemask=/);
    await expect(page).toHaveURL(/antiviral=/);

    // End-to-end: the enabled configs flow through the worker into wasm and
    // the simulation completes without error (the Rust side reads them as
    // `Some(...)` and applies the modifiers).
    await expect(page.getByText(/Ran \d+ simulations/)).toBeVisible({
      timeout: 60_000,
    });
    await expect(page.locator(".error")).toHaveCount(0);
  });
});
