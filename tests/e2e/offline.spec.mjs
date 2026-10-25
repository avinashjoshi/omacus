import { test, expect } from "@playwright/test";
import { mkdirSync, unlinkSync, writeFileSync } from "node:fs";

const genPath = "/tmp/omacus-e2e-sw-gen";
const downPath = "/tmp/omacus-e2e-down";
const shots = "/opt/cursor/artifacts/screenshots";

function generation(n) {
  writeFileSync(genPath, String(n));
}

function unreachable(on) {
  if (on) writeFileSync(downPath, "1");
  else {
    try {
      unlinkSync(downPath);
    } catch {
      // Already reachable.
    }
  }
}

test("first load, offline reload, edit, and an update with unsaved text", async ({ page, context, browserName }) => {
  generation(1);
  unreachable(false);
  mkdirSync(shots, { recursive: true });
  try {
    await page.goto("/app/?sw=1", { waitUntil: "domcontentloaded" });
    await page.waitForSelector("[data-loading]", { state: "detached" });
    await expect(page.locator(".cm-content")).toContainText("Welcome");
    await page.waitForFunction(() => navigator.serviceWorker?.controller);
    await page.waitForFunction(async () => {
      const keys = await caches.keys();
      const name = keys.find((key) => key.startsWith("omacus-app-e2e-"));
      if (!name) return false;
      const urls = [...(await (await caches.open(name)).keys())].map((req) => req.url);
      return urls.some((url) => url.includes("omacus_web_bg.wasm"))
        && urls.some((url) => url.includes("sync-config.js"));
    });

    const editor = page.locator(".cm-content");
    await editor.click();
    await page.keyboard.press("ControlOrMeta+a");
    await page.keyboard.insertText("# Groceries\nmilk = 2 + 2");
    await expect(editor).toContainText("milk = 2 + 2");
    await page.waitForTimeout(400);

    if (browserName === "chromium") {
      await page.evaluate(() => { document.documentElement.dataset.appTheme = "light"; });
      await page.screenshot({ path: `${shots}/app-light.png` });
      await page.evaluate(() => { document.documentElement.dataset.appTheme = "dark"; });
      await page.screenshot({ path: `${shots}/app-dark.png` });
    }

    if (browserName === "chromium") {
      // Airplane mode, and a reload that does not use the HTTP cache.
      await context.setOffline(true);
      const client = await context.newCDPSession(page);
      await client.send("Network.setCacheDisabled", { cacheDisabled: true });
    } else {
      // WebKit in this runner errors on any navigation while emulate-offline is on.
      // Dropping the origin is the same failure the worker has to survive.
      unreachable(true);
    }
    await page.reload({ waitUntil: "domcontentloaded" });
    await page.waitForSelector(".cm-content");
    await expect(page.locator(".cm-content")).toContainText("milk = 2 + 2");
    await page.locator(".cm-content").click();
    await page.keyboard.press("ControlOrMeta+End");
    await page.keyboard.insertText("\nbread = 3");
    await expect(page.locator(".cm-content")).toContainText("bread = 3");
    await page.waitForTimeout(400);
    await page.reload({ waitUntil: "domcontentloaded" });
    await expect(page.locator(".cm-content")).toContainText("bread = 3");
    await expect(page.locator(".cm-content")).toContainText("milk = 2 + 2");
    if (browserName === "chromium") await page.screenshot({ path: `${shots}/app-offline.png` });

    await context.setOffline(false);
    unreachable(false);
    generation(2);
    await page.evaluate(async () => {
      const reg = await navigator.serviceWorker.getRegistration();
      await reg.update();
    });
    const banner = page.locator("[data-update-banner]");
    await expect(banner).toBeVisible();
    await expect(banner).toContainText("Update ready");
    await page.locator(".cm-content").click();
    await page.keyboard.press("ControlOrMeta+End");
    await page.keyboard.insertText("\neggs = 4");
    if (browserName === "chromium") await page.screenshot({ path: `${shots}/app-update.png` });
    await page.locator('[data-action="apply-update"]').click();
    await page.waitForFunction(() => document.querySelector(".cm-content")?.innerText?.includes("eggs = 4"));
    await expect(page.locator(".cm-content")).toContainText("milk = 2 + 2");
    await expect(page.locator(".cm-content")).toContainText("bread = 3");
  } finally {
    unreachable(false);
    await context.setOffline(false).catch(() => {});
    await context.close().catch(() => {});
  }
});
