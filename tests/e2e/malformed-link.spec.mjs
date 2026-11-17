import { test, expect } from "@playwright/test";

const TEXT = "# Trip Budget\nferry = 40\n";

async function seedPage(page) {
  await page.goto("/");
  await page.evaluate((text) => {
    localStorage.setItem("omacus:index", JSON.stringify({
      order: ["trip"],
      active: "trip",
      tree: [{ type: "page", id: "trip" }],
      handles: { trip: "trip" },
      seeded: true,
    }));
    localStorage.setItem("omacus:page:trip", text);
  }, TEXT);
}

async function expectSavedEdit(page) {
  await expect.poll(() => page.evaluate(() => new Promise((resolve, reject) => {
    const request = indexedDB.open("omacus");
    request.onerror = () => reject(request.error);
    request.onsuccess = () => {
      const db = request.result;
      const tx = db.transaction("kv", "readonly");
      const read = tx.objectStore("kv").get("page:trip");
      read.onsuccess = () => resolve(read.result);
      read.onerror = () => reject(read.error);
      tx.oncomplete = () => db.close();
    };
  }))).toBe(TEXT + "train = 25\n");
}

for (const fragment of ["%", "%E0%A4%A", "%FF", "v1.invalid"]) {
  test(`a damaged link #${fragment} keeps the saved page editable`, async ({ page }) => {
    await seedPage(page);
    await page.goto(`/app/#${fragment}`);
    const editor = page.locator(".cm-content");
    await expect(page.locator("[data-loading]")).toHaveCount(0);
    await editor.focus();
    await page.keyboard.press("ControlOrMeta+End");
    await page.keyboard.insertText("train = 25\n");
    await expectSavedEdit(page);
    await expect(page.locator("[data-title]")).toHaveText("Trip Budget");
    await expect(page.locator(".toast")).toContainText("That link couldn't be opened");
    await expect(page).toHaveURL(/\/app\/$/);
    await page.reload();
    await expect(editor).toContainText("ferry = 40");
    await expect(editor).toContainText("train = 25");
    await expect(page.locator("[data-title]")).toHaveText("Trip Budget");
  });
}

test("a damaged link opened while editing keeps unsaved text", async ({ page }) => {
  await seedPage(page);
  await page.goto("/app/");
  const editor = page.locator(".cm-content");
  await expect(editor).toContainText("ferry = 40");
  await editor.focus();
  await page.keyboard.press("ControlOrMeta+End");
  await page.keyboard.insertText("train = 25\n");
  await page.evaluate(() => { location.hash = "%"; });
  await expect(page.locator(".toast")).toContainText("That link couldn't be opened");
  await expect(page).toHaveURL(/\/app\/$/);
  await expect(editor).toContainText("train = 25");
  await expectSavedEdit(page);
  await page.reload();
  await expect(editor).toContainText("ferry = 40");
  await expect(editor).toContainText("train = 25");
});

test("a percent-encoded valid share link still opens the saved copy", async ({ page }) => {
  await seedPage(page);
  await page.goto("/app/");
  await expect(page.locator(".cm-content")).toContainText("ferry = 40");
  const payload = await page.evaluate(async (text) => {
    const { shareEncode } = await import("/app/pkg/omacus_web.js");
    return shareEncode(text);
  }, TEXT);
  await page.goto(`/app/#${payload.replace(/^v/, "%76")}`);
  await expect(page.locator(".cm-content")).toContainText("ferry = 40");
  await expect(page.locator("[data-title]")).toHaveText("Trip Budget");
  await expect(page.locator(".toast")).toContainText("This page is already in your pages.");
  await expect(page).toHaveURL(/\/app\/$/);
});
