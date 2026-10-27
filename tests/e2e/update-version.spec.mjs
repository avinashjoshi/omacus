// A published shell at vN, then a newer shell. The open page asks before it
// reloads, the running copy becomes vN+1, and the pages in IndexedDB stay.

import { test, expect } from "@playwright/test";
import { writeFileSync } from "node:fs";

const genPath = "/tmp/omacus-e2e-sw-gen";

function generation(n) {
  writeFileSync(genPath, String(n));
}

async function pageKeys(page) {
  return page.evaluate(async () => {
    const db = await new Promise((resolve, reject) => {
      const req = indexedDB.open("omacus");
      req.onsuccess = () => resolve(req.result);
      req.onerror = () => reject(req.error);
    });
    try {
      const keys = await new Promise((resolve, reject) => {
        const tx = db.transaction("kv", "readonly");
        const req = tx.objectStore("kv").getAllKeys();
        req.onsuccess = () => resolve(req.result.map(String));
        req.onerror = () => reject(req.error);
      });
      return keys.filter((key) => key.startsWith("page:")).sort();
    } finally {
      db.close();
    }
  });
}

test("an open copy reloads onto the next version and keeps its pages", async ({ page, context }) => {
  generation(1);
  try {
    await page.goto("/app/?sw=1", { waitUntil: "domcontentloaded" });
    await page.waitForSelector("[data-loading]", { state: "detached" });
    await page.waitForFunction(() => navigator.serviceWorker?.controller);
    const before = await page.evaluate(() => document.documentElement.dataset.runningVersion || "");
    expect(before).toContain("-e2e1");

    const editor = page.locator(".cm-content");
    await editor.click();
    await page.keyboard.press("ControlOrMeta+a");
    await page.keyboard.insertText("# Groceries\napples = 3");
    await expect(editor).toContainText("apples = 3");
    await expect.poll(() => pageKeys(page), { timeout: 10000 }).not.toEqual([]);
    const keys = await pageKeys(page);

    generation(2);
    await page.evaluate(() => window.dispatchEvent(new Event("focus")));
    const banner = page.locator("[data-update-banner]");
    await expect(banner).toBeVisible();
    await expect(banner).toContainText("Update ready");
    await page.locator('[data-action="apply-update"]').click();
    await page.waitForFunction((prev) => {
      const running = document.documentElement.dataset.runningVersion || "";
      return running && running !== prev;
    }, before);
    await expect(page.locator(".cm-content")).toContainText("apples = 3");
    const after = await page.evaluate(() => ({
      shell: document.documentElement.dataset.runningVersion || "",
      wasm: document.documentElement.dataset.wasmVersion || "",
    }));
    expect(after.shell).toContain("-e2e2");
    expect(after.wasm.length).toBeGreaterThan(0);
    expect(await pageKeys(page)).toEqual(keys);
    const cachesLeft = await page.evaluate(async () => (await caches.keys()).filter((key) => key.startsWith("omacus-app-")));
    expect(cachesLeft.some((key) => key.endsWith("-e2e2"))).toBe(true);
    expect(cachesLeft.some((key) => key.endsWith("-e2e1"))).toBe(false);
  } finally {
    generation(1);
    await context.close().catch(() => {});
  }
});
