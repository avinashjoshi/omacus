// Settings, which holds Sync, keeps Tab inside and sends Esc back to the opener.
import { expect, test } from "@playwright/test";

test("settings traps focus and escape returns to the opener", async ({ page }) => {
  await page.goto("/app/", { waitUntil: "domcontentloaded" });
  await page.waitForSelector("[data-loading]", { state: "detached" });
  const opener = page.locator("[data-action='settings']:visible");
  await opener.click();
  const dialog = page.locator("[data-settings]");
  await expect(dialog).toBeVisible();
  await expect.poll(() => page.evaluate(() => {
    const root = document.querySelector("[data-settings]");
    return !!(root && !root.hidden && root.contains(document.activeElement));
  })).toBe(true);

  for (let i = 0; i < 30; i += 1) await page.keyboard.press("Tab");
  expect(await page.evaluate(() => document.querySelector("[data-settings]").contains(document.activeElement))).toBe(true);
  for (let i = 0; i < 30; i += 1) await page.keyboard.press("Shift+Tab");
  expect(await page.evaluate(() => document.querySelector("[data-settings]").contains(document.activeElement))).toBe(true);

  await page.keyboard.press("Escape");
  await expect(dialog).toBeHidden();
  await expect(opener).toBeFocused();
});
