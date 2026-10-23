import { test, expect } from "@playwright/test";

test.describe("locking a page that has no password", () => {
  for (const viewport of [
    { label: "mobile", width: 390, height: 844 },
    { label: "desktop", width: 1280, height: 800 },
  ]) {
    test(`${viewport.label}: set a password, then remove the lock`, async ({ page }) => {
      await page.setViewportSize({ width: viewport.width, height: viewport.height });
      await page.goto("/app/", { waitUntil: "domcontentloaded" });
      await page.waitForSelector("[data-loading]", { state: "detached" });
      await page.evaluate(() => {
        document.documentElement.dataset.appTheme = "dark";
      });
      await expect(page.locator(".cm-content")).toContainText("Welcome");

      await page.locator("[data-menu-button]").click();
      await page.getByRole("menuitem", { name: "Lock…" }).click();
      const dialog = page.locator("[data-lock]");
      await expect(dialog).toBeVisible();
      await expect(dialog.locator("[data-lock-detail]")).toContainText("password manager");
      await expect(dialog.locator("[data-lock-confirm]")).toBeVisible();
      await dialog.locator("[data-lock-password]").fill("correct horse");
      await dialog.locator("[data-lock-confirm]").fill("nope");
      await dialog.locator("[data-lock-yes]").click();
      await expect(dialog.locator("[data-lock-error]")).toContainText("don't match");
      await dialog.locator("[data-lock-confirm]").fill("correct horse");
      await dialog.locator("[data-lock-yes]").click();
      await expect(dialog).toBeHidden();
      await expect(page.locator("[data-lock-gate]")).toBeVisible();
      await expect(page.locator(".cm-content")).toBeHidden();

      await page.locator("[data-menu-button]").click();
      await page.getByRole("menuitem", { name: "Remove lock…" }).click();
      await expect(dialog).toBeVisible();
      await expect(dialog.locator("[data-lock-password]")).toBeVisible();
      await dialog.locator("[data-lock-password]").fill("nope");
      await dialog.locator("[data-lock-yes]").click();
      await expect(dialog.locator("[data-lock-error]")).toBeVisible();
      await expect(page.locator("[data-lock-gate]")).toBeVisible();
      await dialog.locator("[data-lock-password]").fill("correct horse");
      await dialog.locator("[data-lock-yes]").click();
      await expect(dialog).toBeHidden();
      await expect(page.locator("[data-lock-gate]")).toBeHidden();
      await expect(page.locator(".cm-content")).toContainText("Welcome");
    });
  }
});
