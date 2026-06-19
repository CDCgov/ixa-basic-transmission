import { test, expect } from "@playwright/test";

// Transmission-modifiers sidebar section: enabling a modifier reveals its
// fields, writes its config to the URL (all three share one grouped
// `modifiers=` param), and round-trips on reload; disabling clears it.

test.describe("Transmission modifiers", () => {
  test("facemask toggles on, reveals fields, round-trips through the URL", async ({
    page,
  }) => {
    await page.goto("/");

    await expect(
      page.getByRole("heading", { name: "Modifiers", exact: true }),
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
    // Modifiers share one grouped URL param.
    await expect(page).toHaveURL(/modifiers=.*facemask/);

    // Reload from the encoded URL → facemask stays enabled.
    await page.goto(page.url());
    await expect(
      page.getByRole("switch", { name: "Facemask" }),
    ).toBeChecked();
    await expect(
      page.getByRole("textbox", { name: "Effectiveness" }),
    ).toBeVisible();

    // Disabling returns modifiers to the default → param drops from the URL.
    await page.getByRole("switch", { name: "Facemask" }).click();
    await expect(page).not.toHaveURL(/modifiers=/);
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

    // Both live under the one grouped `modifiers=` param.
    await expect(page).toHaveURL(/modifiers=/);
    await expect(page).toHaveURL(/facemask/);
    await expect(page).toHaveURL(/antiviral/);

    // End-to-end: the enabled configs flow through the worker into wasm and
    // the simulation completes without error (the Rust side reads them as
    // `Some(...)` and applies the modifiers).
    await expect(page.getByText(/Ran \d+ simulations/)).toBeVisible({
      timeout: 60_000,
    });
    await expect(page.locator(".error")).toHaveCount(0);
  });

  test("isolation needs a setting, then restricts to it and round-trips", async ({
    page,
  }) => {
    await page.goto("/");

    // Disabled until a setting exists (no group to restrict to otherwise).
    const isolation = page.getByRole("switch", { name: "Isolation" });
    await expect(isolation).toBeDisabled();
    await expect(
      page.getByText(/Add a setting above to enable isolation/),
    ).toBeVisible();

    // Add a household setting via the Settings editor.
    await page.getByRole("button", { name: "+ Add setting" }).click();

    // Now isolation can be enabled; restrictTo defaults to the new setting.
    await expect(isolation).toBeEnabled();
    await isolation.click();
    await expect(isolation).toBeChecked();
    await expect(
      page.getByRole("combobox", { name: "Restrict to" }),
    ).toHaveText(/household/);
    await expect(page).toHaveURL(/modifiers=.*isolation/);

    // Reload → isolation stays enabled and targeted at the household setting.
    await page.goto(page.url());
    await expect(
      page.getByRole("switch", { name: "Isolation" }),
    ).toBeChecked();
    await expect(
      page.getByRole("combobox", { name: "Restrict to" }),
    ).toHaveText(/household/);

    // End-to-end: the enabled config flows through wasm and the run completes.
    await expect(page.getByText(/Ran \d+ simulations/)).toBeVisible({
      timeout: 60_000,
    });
    await expect(page.locator(".error")).toHaveCount(0);
  });
});
