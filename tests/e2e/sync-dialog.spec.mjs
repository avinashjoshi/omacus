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
    const card = document.querySelector(".settings-card");
    return !!(card && card.contains(document.activeElement));
  })).toBe(true);

  const scrim = page.locator("button.settings-scrim");
  await expect(scrim).toHaveAttribute("tabindex", "-1");
  await expect(scrim).toHaveAttribute("aria-hidden", "true");

  async function focusStaysInDialog() {
    return page.evaluate(() => {
      const active = document.activeElement;
      const card = document.querySelector(".settings-card");
      return {
        inDialog: !!(card && card.contains(active)),
        scrim: !!(active && active.classList && active.classList.contains("settings-scrim")),
      };
    });
  }
  for (let i = 0; i < 40; i += 1) {
    await page.keyboard.press("Tab");
    const where = await focusStaysInDialog();
    expect(where.scrim).toBe(false);
    expect(where.inDialog).toBe(true);
  }
  for (let i = 0; i < 40; i += 1) {
    await page.keyboard.press("Shift+Tab");
    const where = await focusStaysInDialog();
    expect(where.scrim).toBe(false);
    expect(where.inDialog).toBe(true);
  }

  const box = await scrim.boundingBox();
  expect(box).toBeTruthy();
  // The top bar sits above the backdrop. A click in the lower corner still closes Settings.
  await page.mouse.click(box.x + 8, box.y + box.height - 8);
  await expect(dialog).toBeHidden();
  await opener.click();
  await expect(dialog).toBeVisible();

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
  const field = page.getByRole("textbox", { name: "Recovery words" });
  await expect(page.locator("label[for='sync-recovery-words']")).toHaveText("Recovery words");
  await expect(field).toHaveAttribute("aria-invalid", "true");
  await expect(field).toHaveAttribute("aria-describedby", "sync-field-error");
  await expect(page.locator("#sync-field-error")).toBeVisible();
  await expect(page.locator("[data-sync-note]")).toHaveCount(0);
});
