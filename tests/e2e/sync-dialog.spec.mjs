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

async function openSync(page) {
  await page.goto("/app/", { waitUntil: "domcontentloaded" });
  await page.waitForSelector("[data-loading]", { state: "detached" });
  await page.locator("[data-action='settings']:visible").click();
  await page.locator("[data-settings-tab='sync']").click();
  const setup = page.getByRole("button", { name: "Set up sync" });
  if (await setup.count()) await setup.click();
}

test("an invite error sits on the invite field", async ({ page }) => {
  await openSync(page);
  await page.getByRole("button", { name: "This is my first device" }).click();
  await page.getByRole("button", { name: "Continue" }).click();
  const field = page.locator("[data-access-code]");
  await expect(field).toHaveAttribute("aria-invalid", "true");
  await expect(field).toHaveAttribute("aria-describedby", "sync-field-error");
  await expect(page.locator("#sync-field-error")).toHaveText("Type the invite code you were given.");
  expect(await page.evaluate(() => {
    const input = document.querySelector("[data-access-code]");
    const error = document.getElementById("sync-field-error");
    return input.compareDocumentPosition(error) & Node.DOCUMENT_POSITION_FOLLOWING;
  })).toBeTruthy();
});

test("a join error sits on the join field", async ({ page }) => {
  await openSync(page);
  await page.getByRole("button", { name: "My pages already sync on another device" }).click();
  await page.locator("[data-join-code]").fill("nope");
  await page.getByRole("button", { name: "Join" }).click();
  const field = page.locator("[data-join-code]");
  await expect(field).toHaveAttribute("aria-invalid", "true");
  await expect(field).toHaveAttribute("aria-describedby", "sync-field-error");
  await expect(page.locator("#sync-field-error")).toBeVisible();
  await expect(page.locator(".sync-label + .sync-err")).toHaveCount(0);
});

test("a recovery error sits on the words field", async ({ page }) => {
  await openSync(page);
  await page.getByRole("button", { name: "My pages already sync on another device" }).click();
  await page.getByRole("button", { name: "Use recovery words →" }).click();
  await page.getByRole("button", { name: "Open my pages" }).click();
  const field = page.locator("textarea.sync-phrase");
  await expect(field).toHaveAttribute("aria-invalid", "true");
  await expect(field).toHaveAttribute("aria-describedby", "sync-field-error");
  await expect(page.locator("#sync-field-error")).toBeVisible();
  await expect(page.locator("[data-sync-note]")).toHaveCount(0);
});
